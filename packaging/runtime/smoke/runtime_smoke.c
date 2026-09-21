/* A libadwaita application that proves the private runtime works off its own tree.
 *
 * Four things are asserted, and they are the four that break when a bundled
 * toolkit is wired wrongly:
 *
 *   - libadwaita initialises and a window is presented, which means GTK found
 *     its compiled-in resources, its private GLib and the display;
 *   - a symbolic icon resolves through the bundled Adwaita theme, which means
 *     the icon theme was found at its compiled-in data directory;
 *   - an SVG from that theme loads through gdk-pixbuf, which means the private
 *     loaders.cache was found at its compiled-in path and the private librsvg
 *     loader inside it was loadable;
 *   - a GSettings key reads back, which means the private schema directory and
 *     the private GSettings backend are both in place.
 *
 * Every GLib or GTK warning is fatal, so a missing schema or a missing module
 * fails this program rather than printing a line nobody reads.
 */

#include <adwaita.h>
#include <gdk-pixbuf/gdk-pixbuf.h>
#include <gtk/gtk.h>
#include <string.h>

#define SMOKE_ICON "window-close-symbolic"
#define SMOKE_SVG                                                              \
  "/usr/lib/fermix-desktop/share/icons/Adwaita/symbolic/ui/window-close-symbolic.svg"
#define SMOKE_SCHEMA "org.gtk.gtk4.Settings.FileChooser"

static int exit_code = 1;

#define SMOKE_ICON_DIR "/usr/lib/fermix-desktop/share/icons"

static gboolean check_icon_theme(GtkWidget *window) {
  GtkIconTheme *theme =
      gtk_icon_theme_get_for_display(gtk_widget_get_display(window));

  /* The bundled icon theme is NOT found on its own, and this is the one place
   * the "compiled-in paths need no help" rule does not hold. GTK builds its icon
   * search path from XDG_DATA_DIRS and the user's data directory, not from the
   * prefix it was compiled with, so a private share/icons is invisible until
   * something names it. The application must not set XDG_DATA_DIRS — that would
   * move every other lookup too — so it calls this instead, once, at startup.
   * The smoke does exactly what the application does, and then checks that it
   * worked; the first run of this smoke is what proved the call is required. */
  gtk_icon_theme_add_search_path(theme, SMOKE_ICON_DIR);

  gboolean bundled = FALSE;
  /* Transfer full: GTK hands over a freshly allocated, NULL-terminated vector. */
  char **search = gtk_icon_theme_get_search_path(theme);
  for (int i = 0; search != NULL && search[i] != NULL; i++) {
    if (g_strcmp0(search[i], SMOKE_ICON_DIR) == 0) {
      bundled = TRUE;
    }
  }
  g_strfreev(search);
  if (!bundled) {
    g_printerr("smoke: %s is not on the icon theme search path\n", SMOKE_ICON_DIR);
    return FALSE;
  }

  if (!gtk_icon_theme_has_icon(theme, SMOKE_ICON)) {
    g_printerr("smoke: the bundled icon theme has no %s\n", SMOKE_ICON);
    return FALSE;
  }
  GtkIconPaintable *icon = gtk_icon_theme_lookup_icon(
      theme, SMOKE_ICON, NULL, 16, 1, GTK_TEXT_DIR_LTR,
      GTK_ICON_LOOKUP_FORCE_SYMBOLIC);
  if (icon == NULL) {
    g_printerr("smoke: %s did not resolve to a paintable\n", SMOKE_ICON);
    return FALSE;
  }
  g_object_unref(icon);
  g_print("smoke: icon theme resolved %s\n", SMOKE_ICON);
  return TRUE;
}

static gboolean check_svg_loader(void) {
  GError *error = NULL;
  GdkPixbuf *pixbuf = gdk_pixbuf_new_from_file_at_size(SMOKE_SVG, 16, 16, &error);
  if (pixbuf == NULL) {
    g_printerr("smoke: the SVG loader failed: %s\n",
               error ? error->message : "no error given");
    g_clear_error(&error);
    return FALSE;
  }
  g_print("smoke: SVG decoded to %dx%d\n", gdk_pixbuf_get_width(pixbuf),
          gdk_pixbuf_get_height(pixbuf));
  g_object_unref(pixbuf);
  return TRUE;
}

static gboolean check_gsettings(void) {
  GSettingsSchemaSource *source = g_settings_schema_source_get_default();
  GSettingsSchema *schema =
      source ? g_settings_schema_source_lookup(source, SMOKE_SCHEMA, TRUE) : NULL;
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

/* Assert that a GL renderer really was created, when one was asked for.
 *
 * This is the check whose absence let a missing host library through. The smoke
 * ran with GSK_RENDERER=cairo, which never reaches libepoxy's dlopen of libGL,
 * libEGL and libGLESv2 — so every one of them could have been absent and this
 * program would still have printed success. It took slice 4's package aborting
 * under Xvfb to find it.
 *
 * GTK falls back to cairo silently when GL cannot be set up, which is right for
 * a user and useless for a test: a fallback here would turn the whole point of
 * the run into a pass. So when SMOKE_EXPECT_GL is set, the renderer's type name
 * must contain "Gl" and must not be the cairo one. */
static gboolean check_renderer(GtkWidget *window) {
  const char *expect = g_getenv("SMOKE_EXPECT_GL");
  GskRenderer *renderer = gtk_native_get_renderer(GTK_NATIVE(window));
  const char *name = renderer ? G_OBJECT_TYPE_NAME(renderer) : "none";

  g_print("smoke: renderer %s\n", name);
  if (expect == NULL || *expect == '\0') {
    return TRUE;
  }
  /* GTK 4.16 names it GskGLRenderer, with GL capitalised; an earlier spelling
   * of this check looked for "Gl" and failed against a renderer that was
   * working perfectly. Both spellings are accepted, and the cairo renderer is
   * named explicitly, so this cannot pass on a fallback whatever GTK calls its
   * GL renderer next. */
  if (renderer == NULL || strstr(name, "Cairo") != NULL ||
      (strstr(name, "GL") == NULL && strstr(name, "Gl") == NULL)) {
    g_printerr("smoke: a GL renderer was asked for and %s was created;"
               " GTK fell back, which means the GL path never ran\n", name);
    return FALSE;
  }
  return TRUE;
}

static void on_activate(GtkApplication *app, gpointer user_data) {
  (void)user_data;

  GtkWidget *window = adw_application_window_new(app);
  gtk_window_set_default_size(GTK_WINDOW(window), 320, 200);
  gtk_window_set_title(GTK_WINDOW(window), "fermix runtime smoke");

  GtkWidget *view = adw_toolbar_view_new();
  adw_toolbar_view_add_top_bar(ADW_TOOLBAR_VIEW(view), adw_header_bar_new());
  adw_toolbar_view_set_content(ADW_TOOLBAR_VIEW(view), adw_status_page_new());
  adw_application_window_set_content(ADW_APPLICATION_WINDOW(window), view);
  gtk_window_present(GTK_WINDOW(window));

  if (check_renderer(window) && check_icon_theme(window) && check_svg_loader() &&
      check_gsettings()) {
    g_print("smoke: adwaita %d.%d.%d, gtk %d.%d.%d\n", ADW_MAJOR_VERSION,
            ADW_MINOR_VERSION, ADW_MICRO_VERSION, gtk_get_major_version(),
            gtk_get_minor_version(), gtk_get_micro_version());
    exit_code = 0;
  }
  g_application_quit(G_APPLICATION(app));
}

int main(int argc, char **argv) {
  g_log_set_always_fatal(G_LOG_LEVEL_WARNING | G_LOG_LEVEL_CRITICAL);

  /* No application id. A throwaway probe has no business owning a bus name,
   * and scripts/check_app_identity.sh holds that exactly one io.tezra.Fermix*
   * identity exists in this tree — the application's, and not this one's. */
  AdwApplication *app = adw_application_new(NULL, G_APPLICATION_NON_UNIQUE);
  g_signal_connect(app, "activate", G_CALLBACK(on_activate), NULL);
  int status = g_application_run(G_APPLICATION(app), argc, argv);
  g_object_unref(app);

  return status != 0 ? status : exit_code;
}
