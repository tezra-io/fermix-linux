/* A libadwaita window that proves the private runtime works off its own tree.
 *
 * It presents an AdwApplicationWindow showing a PNG and a WebP texture, waits
 * for it to draw, and then checks what breaks when a bundled toolkit is wired
 * wrongly:
 *
 *   - the renderer is the one asked for (a GL pass must not fall back to cairo);
 *   - a symbolic icon resolves through the bundled Adwaita theme;
 *   - a GSettings key reads back through the private schemas and backend;
 *   - every vendor mark decodes the way desktop/app/src/marks.rs decodes it:
 *     GTK's own SVG renderer, the image loader where GTK reports a missing SVG
 *     feature, and the image loader for every raster.
 *
 * It saves a screenshot of the window and quits by itself. Every GLib or GTK
 * warning is fatal, so a missing schema, module or loader fails the run.
 *
 * Inputs, from the environment: SMOKE_MARKS (a directory), SMOKE_PNG and
 * SMOKE_WEBP (files), SMOKE_SCREENSHOT (the PNG to write) and SMOKE_EXPECT_GL.
 */

#include <adwaita.h>
#include <gtk/gtk.h>
#include <string.h>

#define SMOKE_ICON_DIR "/usr/lib/fermix-desktop/share/icons"
#define SMOKE_ICON "window-close-symbolic"
#define SMOKE_SCHEMA "org.gtk.gtk4.Settings.FileChooser"
#define SMOKE_SETTLE_FRAMES 3
#define SMOKE_TIMEOUT_SECONDS 30
#define SMOKE_MAX_MARKS 200
#define SMOKE_MAX_DEPTH 4

typedef struct {
  const char *marks;
  const char *png;
  const char *webp;
  const char *screenshot;
  gboolean expect_gl;
  GtkApplication *app;
  GtkWidget *window;
  guint watchdog;
  int frames;
  int exit_code;
} Smoke;

typedef struct {
  int decoded;
  int failed;
} MarkCount;

static const char *required_env(const char *name) {
  const char *value = g_getenv(name);
  if (value == NULL || *value == '\0') {
    g_printerr("smoke: %s is not set\n", name);
    return NULL;
  }
  return value;
}

/* The renderer GTK chose. GTK falls back to cairo when GL cannot be set up,
 * which is right for a user and would make a GL pass pass without GL. */
static gboolean check_renderer(Smoke *smoke) {
  GskRenderer *renderer = gtk_native_get_renderer(GTK_NATIVE(smoke->window));
  const char *name = renderer ? G_OBJECT_TYPE_NAME(renderer) : "none";

  g_print("smoke: renderer %s\n", name);
  if (!smoke->expect_gl) {
    return TRUE;
  }
  if (renderer == NULL || strstr(name, "Cairo") != NULL || strstr(name, "GL") == NULL) {
    g_printerr("smoke: a GL renderer was asked for and %s was created\n", name);
    return FALSE;
  }
  return TRUE;
}

/* GTK builds its icon search path from XDG_DATA_DIRS, not from its prefix, so
 * the bundled theme is appended here exactly as the application appends it. */
static gboolean check_icon_theme(Smoke *smoke) {
  GtkIconTheme *theme = gtk_icon_theme_get_for_display(gtk_widget_get_display(smoke->window));

  gtk_icon_theme_add_search_path(theme, SMOKE_ICON_DIR);
  if (!gtk_icon_theme_has_icon(theme, SMOKE_ICON)) {
    g_printerr("smoke: the bundled icon theme has no %s\n", SMOKE_ICON);
    return FALSE;
  }
  GtkIconPaintable *icon = gtk_icon_theme_lookup_icon(theme, SMOKE_ICON, NULL, 16, 1,
                                                      GTK_TEXT_DIR_LTR, 0);
  if (icon == NULL) {
    g_printerr("smoke: %s did not resolve\n", SMOKE_ICON);
    return FALSE;
  }
  g_object_unref(icon);
  g_print("smoke: icon theme resolved %s\n", SMOKE_ICON);
  return TRUE;
}

static gboolean check_gsettings(void) {
  GSettingsSchemaSource *source = g_settings_schema_source_get_default();
  GSettingsSchema *schema = source ? g_settings_schema_source_lookup(source, SMOKE_SCHEMA, TRUE) : NULL;
  if (schema == NULL) {
    g_printerr("smoke: the private GSettings schemas were not found\n");
    return FALSE;
  }
  g_settings_schema_unref(schema);

  GSettings *settings = g_settings_new(SMOKE_SCHEMA);
  char *value = g_settings_get_string(settings, "sort-column");
  g_print("smoke: GSettings %s sort-column = %s\n", SMOKE_SCHEMA, value);
  g_free(value);
  g_object_unref(settings);
  return TRUE;
}

static void on_svg_error(GtkSvg *svg, GError *error, gpointer data) {
  gboolean *lacking = data;
  (void)svg;
  if (g_error_matches(error, GTK_SVG_ERROR, GTK_SVG_ERROR_NOT_IMPLEMENTED)) {
    *lacking = TRUE;
  }
}

/* How the window would draw the file, or NULL when it could not. */
static const char *decode_mark(const char *path, GBytes *bytes) {
  if (g_str_has_suffix(path, ".svg")) {
    gboolean lacking = FALSE;
    GtkSvg *svg = gtk_svg_new();
    gulong handler = g_signal_connect(svg, "error", G_CALLBACK(on_svg_error), &lacking);
    gtk_svg_load_from_bytes(svg, bytes);
    g_signal_handler_disconnect(svg, handler);
    g_object_unref(svg);
    if (!lacking) {
      return "GtkSvg";
    }
  }
  GdkTexture *texture = gdk_texture_new_from_bytes(bytes, NULL);
  if (texture == NULL) {
    return NULL;
  }
  g_object_unref(texture);
  return g_str_has_suffix(path, ".svg") ? "image loader, a feature GtkSvg lacks" : "image loader";
}

static gboolean is_mark(const char *name) {
  return g_str_has_suffix(name, ".svg") || g_str_has_suffix(name, ".png") ||
         g_str_has_suffix(name, ".webp");
}

static void count_mark(const char *path, MarkCount *count) {
  GError *error = NULL;
  char *contents = NULL;
  gsize length = 0;
  if (!g_file_get_contents(path, &contents, &length, &error)) {
    g_printerr("smoke: %s: %s\n", path, error->message);
    g_clear_error(&error);
    count->failed++;
    return;
  }
  GBytes *bytes = g_bytes_new_take(contents, length);
  const char *how = decode_mark(path, bytes);
  g_bytes_unref(bytes);
  g_print("smoke: mark %s: %s\n", path, how ? how : "DID NOT DECODE");
  if (how == NULL) {
    count->failed++;
  } else {
    count->decoded++;
  }
}

/* Bounded in depth and in count; reaching either cap fails the check, since a
 * mark past it would go unchecked. */
static void walk_marks(const char *dir, int depth, MarkCount *count) {
  if (depth > SMOKE_MAX_DEPTH) {
    g_printerr("smoke: %s is deeper than %d directories\n", dir, SMOKE_MAX_DEPTH);
    count->failed++;
    return;
  }
  GDir *handle = g_dir_open(dir, 0, NULL);
  if (handle == NULL) {
    g_printerr("smoke: %s cannot be read\n", dir);
    count->failed++;
    return;
  }
  const char *name;
  while ((name = g_dir_read_name(handle)) != NULL && count->decoded + count->failed < SMOKE_MAX_MARKS) {
    char *path = g_build_filename(dir, name, NULL);
    if (g_file_test(path, G_FILE_TEST_IS_DIR)) {
      walk_marks(path, depth + 1, count);
    } else if (is_mark(name)) {
      count_mark(path, count);
    }
    g_free(path);
  }
  g_dir_close(handle);
}

static gboolean check_marks(Smoke *smoke) {
  MarkCount count = {0, 0};
  walk_marks(smoke->marks, 0, &count);
  g_print("smoke: %d marks decoded, %d did not\n", count.decoded, count.failed);
  if (count.decoded == 0 || count.failed != 0 || count.decoded >= SMOKE_MAX_MARKS) {
    g_printerr("smoke: the marks under %s do not all decode\n", smoke->marks);
    return FALSE;
  }
  return TRUE;
}

static gboolean save_screenshot(Smoke *smoke) {
  GdkPaintable *paintable = gtk_widget_paintable_new(smoke->window);
  int width = gdk_paintable_get_intrinsic_width(paintable);
  int height = gdk_paintable_get_intrinsic_height(paintable);
  GtkSnapshot *snapshot = gtk_snapshot_new();
  gdk_paintable_snapshot(paintable, GDK_SNAPSHOT(snapshot), width, height);
  GskRenderNode *node = gtk_snapshot_free_to_node(snapshot);
  gboolean saved = FALSE;
  if (node != NULL && width > 0 && height > 0) {
    GskRenderer *renderer = gtk_native_get_renderer(GTK_NATIVE(smoke->window));
    graphene_rect_t viewport = GRAPHENE_RECT_INIT(0, 0, width, height);
    GdkTexture *texture = gsk_renderer_render_texture(renderer, node, &viewport);
    saved = gdk_texture_save_to_png(texture, smoke->screenshot);
    g_object_unref(texture);
  }
  g_clear_pointer(&node, gsk_render_node_unref);
  g_object_unref(paintable);
  g_print("smoke: screenshot %dx%d %s %s\n", width, height, saved ? "saved to" : "NOT saved to",
          smoke->screenshot);
  return saved;
}

static void finish(Smoke *smoke) {
  g_clear_handle_id(&smoke->watchdog, g_source_remove);
  if (check_renderer(smoke) && check_icon_theme(smoke) && check_gsettings() && check_marks(smoke) &&
      save_screenshot(smoke)) {
    g_print("smoke: adwaita %d.%d.%d, gtk %u.%u.%u\n", ADW_MAJOR_VERSION, ADW_MINOR_VERSION,
            ADW_MICRO_VERSION, gtk_get_major_version(), gtk_get_minor_version(),
            gtk_get_micro_version());
    smoke->exit_code = 0;
  }
  g_application_quit(G_APPLICATION(smoke->app));
}

/* A few frames after the window first draws, so the screenshot shows it. */
static gboolean on_tick(GtkWidget *widget, GdkFrameClock *clock, gpointer data) {
  Smoke *smoke = data;
  (void)widget;
  (void)clock;
  smoke->frames++;
  if (smoke->frames < SMOKE_SETTLE_FRAMES) {
    return G_SOURCE_CONTINUE;
  }
  finish(smoke);
  return G_SOURCE_REMOVE;
}

static gboolean on_watchdog(gpointer data) {
  Smoke *smoke = data;
  smoke->watchdog = 0;
  g_printerr("smoke: the window drew %d frames in %d seconds\n", smoke->frames, SMOKE_TIMEOUT_SECONDS);
  g_application_quit(G_APPLICATION(smoke->app));
  return G_SOURCE_REMOVE;
}

static GtkWidget *texture_picture(const char *path) {
  GError *error = NULL;
  GdkTexture *texture = gdk_texture_new_from_filename(path, &error);
  if (texture == NULL) {
    g_printerr("smoke: %s did not load: %s\n", path, error->message);
    g_clear_error(&error);
    return NULL;
  }
  g_print("smoke: texture %s %dx%d\n", path, gdk_texture_get_width(texture),
          gdk_texture_get_height(texture));
  GtkWidget *picture = gtk_picture_new_for_paintable(GDK_PAINTABLE(texture));
  gtk_widget_set_size_request(picture, 96, 96);
  g_object_unref(texture);
  return picture;
}

static void on_activate(GtkApplication *app, gpointer data) {
  Smoke *smoke = data;
  GtkWidget *png = texture_picture(smoke->png);
  GtkWidget *webp = texture_picture(smoke->webp);
  if (png == NULL || webp == NULL) {
    g_application_quit(G_APPLICATION(app));
    return;
  }
  GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 24);
  gtk_widget_set_halign(row, GTK_ALIGN_CENTER);
  gtk_widget_set_valign(row, GTK_ALIGN_CENTER);
  gtk_box_append(GTK_BOX(row), png);
  gtk_box_append(GTK_BOX(row), webp);
  gtk_box_append(GTK_BOX(row), gtk_image_new_from_icon_name(SMOKE_ICON));

  GtkWidget *view = adw_toolbar_view_new();
  adw_toolbar_view_add_top_bar(ADW_TOOLBAR_VIEW(view), adw_header_bar_new());
  adw_toolbar_view_set_content(ADW_TOOLBAR_VIEW(view), row);

  smoke->window = adw_application_window_new(app);
  gtk_window_set_default_size(GTK_WINDOW(smoke->window), 420, 260);
  gtk_window_set_title(GTK_WINDOW(smoke->window), "fermix runtime smoke");
  adw_application_window_set_content(ADW_APPLICATION_WINDOW(smoke->window), view);
  smoke->watchdog = g_timeout_add_seconds(SMOKE_TIMEOUT_SECONDS, on_watchdog, smoke);
  gtk_widget_add_tick_callback(smoke->window, on_tick, smoke, NULL);
  gtk_window_present(GTK_WINDOW(smoke->window));
}

int main(int argc, char **argv) {
  Smoke smoke = {0};
  g_log_set_always_fatal(G_LOG_LEVEL_WARNING | G_LOG_LEVEL_CRITICAL);
  smoke.marks = required_env("SMOKE_MARKS");
  smoke.png = required_env("SMOKE_PNG");
  smoke.webp = required_env("SMOKE_WEBP");
  smoke.screenshot = required_env("SMOKE_SCREENSHOT");
  if (!smoke.marks || !smoke.png || !smoke.webp || !smoke.screenshot) {
    return 2;
  }
  smoke.expect_gl = g_strcmp0(g_getenv("SMOKE_EXPECT_GL"), "1") == 0;
  smoke.exit_code = 1;

  /* No application id: a probe has no business owning a bus name. */
  AdwApplication *app = adw_application_new(NULL, G_APPLICATION_NON_UNIQUE);
  smoke.app = GTK_APPLICATION(app);
  g_signal_connect(app, "activate", G_CALLBACK(on_activate), &smoke);
  int status = g_application_run(G_APPLICATION(app), argc, argv);
  g_object_unref(app);
  return status != 0 ? status : smoke.exit_code;
}
