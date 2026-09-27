// The stage and scene behind fermix_rive.h: an offscreen EGL context, the Rive
// GL renderer in it, and the pixels read back after each frame.

#include "fermix_rive.h"

#include <EGL/egl.h>
#include <EGL/eglext.h>

#include "rive/animation/state_machine_instance.hpp"
#include "rive/artboard.hpp"
#include "rive/file.hpp"
#include "rive/renderer/gl/gles3.hpp"
#include "rive/renderer/gl/render_context_gl_impl.hpp"
#include "rive/renderer/gl/render_target_gl.hpp"
#include "rive/renderer/rive_renderer.hpp"
#include "rive/viewmodel/runtime/viewmodel_instance_enum_runtime.hpp"
#include "rive/viewmodel/runtime/viewmodel_instance_number_runtime.hpp"
#include "rive/viewmodel/runtime/viewmodel_instance_runtime.hpp"
#include "rive/viewmodel/runtime/viewmodel_runtime.hpp"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

using rive::gpu::FramebufferRenderTargetGL;
using rive::gpu::RenderContext;
using rive::gpu::RenderContextGLImpl;

struct FxStage
{
    EGLDisplay display = EGL_NO_DISPLAY;
    EGLContext context = EGL_NO_CONTEXT;
    EGLenum api = EGL_OPENGL_API;
    // Kept for the file's lifetime: the import reads from it.
    std::vector<uint8_t> riv;
    std::unique_ptr<RenderContext> render;
    rive::rcp<rive::File> file;
};

struct FxScene
{
    std::unique_ptr<rive::ArtboardInstance> artboard;
    std::unique_ptr<rive::StateMachineInstance> machine;
    rive::rcp<rive::ViewModelInstanceRuntime> model;
    // The offscreen target, remade when the size changes.
    GLuint framebuffer = 0;
    GLuint texture = 0;
    uint32_t width = 0;
    uint32_t height = 0;
    rive::rcp<FramebufferRenderTargetGL> target;
};

// The most devices tried on a driver that offers devices rather than Mesa's
// surfaceless platform.
static constexpr EGLint kMaxDevices = 8;

// The shim's preconditions, checked in every build. FX_REQUIRE() cannot be used:
// the whole runtime is built with NDEBUG, because Rive's class layouts follow
// it, and every file that includes Rive's headers must agree.
#define FX_REQUIRE(condition)                                                  \
    ((condition) ? (void)0 : fx_abort(#condition, __FILE__, __LINE__))

[[noreturn]] static void fx_abort(const char* condition, const char* file, int line)
{
    fprintf(stderr, "fermix-rive: %s:%d: %s does not hold\n", file, line, condition);
    abort();
}

// A failure a free function cannot hand back, said where the app's log sees it.
static void warn(const char* sentence) { fprintf(stderr, "fermix-rive: %s\n", sentence); }

static void fail(char* error, size_t error_len, const std::string& sentence)
{
    FX_REQUIRE(error != nullptr && error_len > 0);
    snprintf(error, error_len, "%s", sentence.c_str());
}

// ---- The thread's GL context, saved and put back ----

struct Saved
{
    EGLenum api;
    EGLDisplay display;
    EGLSurface draw;
    EGLSurface read;
    EGLContext context;
};

static Saved save_current()
{
    return Saved{eglQueryAPI(),
                 eglGetCurrentDisplay(),
                 eglGetCurrentSurface(EGL_DRAW),
                 eglGetCurrentSurface(EGL_READ),
                 eglGetCurrentContext()};
}

static bool make_current(const FxStage& stage)
{
    return eglBindAPI(stage.api) &&
           eglMakeCurrent(stage.display, EGL_NO_SURFACE, EGL_NO_SURFACE, stage.context);
}

// Puts back the context the thread had, or none. The stage's context is
// released either way, so it is never left current behind the caller's back.
static bool restore_current(const Saved& saved, EGLDisplay stage_display)
{
    if (saved.context == EGL_NO_CONTEXT)
    {
        bool released =
            eglMakeCurrent(stage_display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        return eglBindAPI(saved.api) && released;
    }
    return eglBindAPI(saved.api) &&
           eglMakeCurrent(saved.display, saved.draw, saved.read, saved.context);
}

// ---- The stage ----

static bool initialize(EGLDisplay display)
{
    return display != EGL_NO_DISPLAY && eglInitialize(display, nullptr, nullptr);
}

// Mesa's surfaceless platform first: every Mesa driver, and llvmpipe where
// there is no GPU. Then the devices another vendor's EGL offers (NVIDIA).
static EGLDisplay open_display()
{
    auto get_display = reinterpret_cast<PFNEGLGETPLATFORMDISPLAYEXTPROC>(
        eglGetProcAddress("eglGetPlatformDisplayEXT"));
    if (get_display == nullptr)
    {
        return EGL_NO_DISPLAY;
    }
    EGLDisplay surfaceless =
        get_display(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, nullptr);
    if (initialize(surfaceless))
    {
        return surfaceless;
    }
    auto query_devices =
        reinterpret_cast<PFNEGLQUERYDEVICESEXTPROC>(eglGetProcAddress("eglQueryDevicesEXT"));
    EGLDeviceEXT devices[kMaxDevices];
    EGLint count = 0;
    if (query_devices == nullptr || !query_devices(kMaxDevices, devices, &count))
    {
        return EGL_NO_DISPLAY;
    }
    for (EGLint i = 0; i < std::min(count, kMaxDevices); ++i)
    {
        EGLDisplay device = get_display(EGL_PLATFORM_DEVICE_EXT, devices[i], nullptr);
        if (initialize(device))
        {
            return device;
        }
    }
    return EGL_NO_DISPLAY;
}

// Desktop GL 4.2, the renderer's floor there, then OpenGL ES 3.0, its floor there.
static bool create_context(FxStage* stage)
{
    const EGLint desktop[] = {EGL_CONTEXT_MAJOR_VERSION,
                              4,
                              EGL_CONTEXT_MINOR_VERSION,
                              2,
                              EGL_CONTEXT_OPENGL_PROFILE_MASK,
                              EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
                              EGL_NONE};
    const EGLint embedded[] = {EGL_CONTEXT_MAJOR_VERSION, 3, EGL_NONE};
    const struct
    {
        EGLenum api;
        const EGLint* attributes;
    } attempts[] = {{EGL_OPENGL_API, desktop}, {EGL_OPENGL_ES_API, embedded}};
    for (const auto& attempt : attempts)
    {
        if (!eglBindAPI(attempt.api))
        {
            continue;
        }
        stage->context = eglCreateContext(stage->display,
                                          EGL_NO_CONFIG_KHR,
                                          EGL_NO_CONTEXT,
                                          attempt.attributes);
        if (stage->context != EGL_NO_CONTEXT)
        {
            stage->api = attempt.api;
            return true;
        }
    }
    return false;
}

// With the stage's context current: GL, the renderer, then the file.
static bool fill_stage(FxStage* stage, char* error, size_t error_len)
{
    if (!gladLoadCustomLoader(reinterpret_cast<GLADloadfunc>(eglGetProcAddress)))
    {
        fail(error, error_len, "OpenGL could not be loaded");
        return false;
    }
    stage->render = RenderContextGLImpl::MakeContext();
    if (!stage->render)
    {
        fail(error,
             error_len,
             std::string("the Rive renderer cannot run on ") +
                 reinterpret_cast<const char*>(glGetString(GL_RENDERER)));
        return false;
    }
    rive::ImportResult result = rive::ImportResult::success;
    stage->file = rive::File::import(stage->riv, stage->render.get(), &result);
    if (!stage->file)
    {
        fail(error,
             error_len,
             "the animation file could not be read (import result " +
                 std::to_string(static_cast<int>(result)) + ")");
        return false;
    }
    return true;
}

static void release_stage(FxStage* stage)
{
    if (stage->context != EGL_NO_CONTEXT)
    {
        Saved saved = save_current();
        bool current = make_current(*stage);
        if (!current)
        {
            warn("the offscreen OpenGL context could not be made current to free the stage");
        }
        // Destroying the context frees its GL objects either way.
        stage->file = nullptr;
        stage->render = nullptr;
        if (current && !restore_current(saved, stage->display))
        {
            warn("the previous OpenGL context could not be made current again");
        }
        eglDestroyContext(stage->display, stage->context);
    }
    // The display is not terminated: an EGL display is one per platform and
    // process, shared with every other context on it, so the stage owns only
    // its context.
    delete stage;
}

extern "C" FxStage* fx_stage_new(const uint8_t* riv,
                                 size_t riv_len,
                                 char* error,
                                 size_t error_len)
{
    FX_REQUIRE(riv != nullptr && riv_len > 0);
    auto* stage = new FxStage();
    stage->riv.assign(riv, riv + riv_len);
    stage->display = open_display();
    if (stage->display == EGL_NO_DISPLAY)
    {
        fail(error, error_len, "no EGL display can draw offscreen");
        release_stage(stage);
        return nullptr;
    }
    if (!create_context(stage))
    {
        fail(error, error_len, "no OpenGL 4.2 or OpenGL ES 3.0 context could be made");
        release_stage(stage);
        return nullptr;
    }
    Saved saved = save_current();
    if (!make_current(*stage))
    {
        fail(error, error_len, "the offscreen OpenGL context could not be made current");
        release_stage(stage);
        return nullptr;
    }
    bool filled = fill_stage(stage, error, error_len);
    bool restored = restore_current(saved, stage->display);
    if (filled && !restored)
    {
        fail(error, error_len, "the previous OpenGL context could not be made current again");
    }
    if (!filled || !restored)
    {
        release_stage(stage);
        return nullptr;
    }
    return stage;
}

extern "C" void fx_stage_free(FxStage* stage)
{
    FX_REQUIRE(stage != nullptr);
    release_stage(stage);
}

// ---- Scenes ----

static bool fill_scene(FxStage* stage,
                       FxScene* scene,
                       const char* state_machine,
                       char* error,
                       size_t error_len)
{
    scene->artboard = stage->file->artboardDefault();
    if (!scene->artboard)
    {
        fail(error, error_len, "the animation has no artboard");
        return false;
    }
    scene->machine = scene->artboard->stateMachineNamed(state_machine);
    if (!scene->machine)
    {
        fail(error,
             error_len,
             std::string("the animation has no state machine named ") + state_machine);
        return false;
    }
    rive::ViewModelRuntime* model = stage->file->defaultArtboardViewModel(scene->artboard.get());
    scene->model = model != nullptr ? model->createDefaultInstance() : nullptr;
    if (!scene->model)
    {
        fail(error, error_len, "the animation's artboard has no view model instance");
        return false;
    }
    scene->machine->bindViewModelInstance(scene->model->instance());
    scene->machine->advanceAndApply(0.0f);
    return true;
}

static void release_target(FxScene* scene)
{
    scene->target = nullptr;
    glDeleteFramebuffers(1, &scene->framebuffer);
    glDeleteTextures(1, &scene->texture);
    scene->framebuffer = 0;
    scene->texture = 0;
    scene->width = 0;
    scene->height = 0;
}

extern "C" FxScene* fx_scene_new(FxStage* stage,
                                 const char* state_machine,
                                 char* error,
                                 size_t error_len)
{
    FX_REQUIRE(stage != nullptr && state_machine != nullptr);
    auto* scene = new FxScene();
    if (!fill_scene(stage, scene, state_machine, error, error_len))
    {
        delete scene;
        return nullptr;
    }
    return scene;
}

// The scene's framebuffer and texture, freed with the stage's context current.
static void free_target(FxStage* stage, FxScene* scene)
{
    Saved saved = save_current();
    if (!make_current(*stage))
    {
        warn("the offscreen OpenGL context could not be made current to free a scene");
        return;
    }
    release_target(scene);
    if (!restore_current(saved, stage->display))
    {
        warn("the previous OpenGL context could not be made current again");
    }
}

extern "C" void fx_scene_free(FxStage* stage, FxScene* scene)
{
    FX_REQUIRE(stage != nullptr && scene != nullptr);
    if (scene->framebuffer != 0)
    {
        free_target(stage, scene);
    }
    delete scene;
}

extern "C" bool fx_scene_set_enum(FxScene* scene, const char* property, const char* value)
{
    FX_REQUIRE(scene != nullptr && property != nullptr && value != nullptr);
    rive::ViewModelInstanceEnumRuntime* field = scene->model->propertyEnum(property);
    if (field == nullptr)
    {
        return false;
    }
    std::vector<std::string> values = field->values();
    if (std::find(values.begin(), values.end(), value) == values.end())
    {
        return false;
    }
    field->value(value);
    return true;
}

extern "C" bool fx_scene_set_number(FxScene* scene, const char* property, float value)
{
    FX_REQUIRE(scene != nullptr && property != nullptr);
    rive::ViewModelInstanceNumberRuntime* field = scene->model->propertyNumber(property);
    if (field == nullptr)
    {
        return false;
    }
    field->value(value);
    return true;
}

extern "C" void fx_scene_advance(FxScene* scene, float seconds)
{
    FX_REQUIRE(scene != nullptr && seconds >= 0.0f);
    scene->machine->advanceAndApply(seconds);
}

// ---- Drawing ----

// A texture-backed framebuffer the size of the frame, made when the size changes.
static bool ensure_target(FxScene* scene, uint32_t width, uint32_t height)
{
    if (scene->framebuffer != 0 && scene->width == width && scene->height == height)
    {
        return true;
    }
    if (scene->framebuffer != 0)
    {
        release_target(scene);
    }
    glGenTextures(1, &scene->texture);
    glBindTexture(GL_TEXTURE_2D, scene->texture);
    glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, width, height);
    glGenFramebuffers(1, &scene->framebuffer);
    glBindFramebuffer(GL_FRAMEBUFFER, scene->framebuffer);
    glFramebufferTexture2D(GL_FRAMEBUFFER,
                           GL_COLOR_ATTACHMENT0,
                           GL_TEXTURE_2D,
                           scene->texture,
                           0);
    if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    {
        release_target(scene);
        return false;
    }
    scene->width = width;
    scene->height = height;
    scene->target = rive::make_rcp<FramebufferRenderTargetGL>(width, height, scene->framebuffer, 1);
    return true;
}

// GL reads the bottom row first; the caller wants the top row first.
static void flip_rows(uint8_t* rgba, uint32_t width, uint32_t height)
{
    const size_t stride = static_cast<size_t>(width) * 4;
    std::vector<uint8_t> row(stride);
    for (uint32_t top = 0, bottom = height - 1; top < bottom; ++top, --bottom)
    {
        std::memcpy(row.data(), rgba + top * stride, stride);
        std::memcpy(rgba + top * stride, rgba + bottom * stride, stride);
        std::memcpy(rgba + bottom * stride, row.data(), stride);
    }
}

// The renderer dithers gradients, which can leave a channel a step above its
// pixel's alpha where a soft edge fades out. A caller reading premultiplied
// pixels must never see that, so every channel is held to its alpha.
static void clamp_to_alpha(uint8_t* rgba, uint32_t width, uint32_t height)
{
    const size_t pixels = static_cast<size_t>(width) * height;
    for (size_t i = 0; i < pixels; ++i)
    {
        uint8_t* px = rgba + i * 4;
        px[0] = std::min(px[0], px[3]);
        px[1] = std::min(px[1], px[3]);
        px[2] = std::min(px[2], px[3]);
    }
}

// With the stage's context current.
static bool draw(FxStage* stage,
                 FxScene* scene,
                 uint32_t width,
                 uint32_t height,
                 uint8_t* rgba,
                 char* error,
                 size_t error_len)
{
    if (!ensure_target(scene, width, height))
    {
        fail(error, error_len, "the offscreen framebuffer is incomplete");
        return false;
    }
    auto* gl = stage->render->static_impl_cast<RenderContextGLImpl>();
    gl->invalidateGLState();
    RenderContext::FrameDescriptor frame;
    frame.renderTargetWidth = width;
    frame.renderTargetHeight = height;
    frame.loadAction = rive::gpu::LoadAction::clear;
    frame.clearColor = 0;
    stage->render->beginFrame(frame);
    rive::RiveRenderer renderer(stage->render.get());
    renderer.save();
    renderer.align(rive::Fit::contain,
                   rive::Alignment::center,
                   rive::AABB(0.0f, 0.0f, static_cast<float>(width), static_cast<float>(height)),
                   scene->artboard->bounds());
    scene->artboard->draw(&renderer);
    renderer.restore();
    RenderContext::FlushResources flush;
    flush.renderTarget = scene->target.get();
    stage->render->flush(flush);
    gl->unbindGLInternalResources();
    glBindFramebuffer(GL_READ_FRAMEBUFFER, scene->framebuffer);
    glReadPixels(0, 0, width, height, GL_RGBA, GL_UNSIGNED_BYTE, rgba);
    GLenum status = glGetError();
    if (status != GL_NO_ERROR)
    {
        fail(error, error_len, "reading the frame back failed (GL error " + std::to_string(status) + ")");
        return false;
    }
    flip_rows(rgba, width, height);
    clamp_to_alpha(rgba, width, height);
    return true;
}

extern "C" bool fx_scene_render(FxStage* stage,
                                FxScene* scene,
                                uint32_t width,
                                uint32_t height,
                                uint8_t* rgba,
                                size_t rgba_len,
                                char* error,
                                size_t error_len)
{
    FX_REQUIRE(stage != nullptr && scene != nullptr && rgba != nullptr);
    FX_REQUIRE(width > 0 && height > 0 && rgba_len == static_cast<size_t>(width) * height * 4);
    Saved saved = save_current();
    if (!make_current(*stage))
    {
        fail(error, error_len, "the offscreen OpenGL context could not be made current");
        return false;
    }
    bool drawn = draw(stage, scene, width, height, rgba, error, error_len);
    if (!restore_current(saved, stage->display))
    {
        fail(error, error_len, "the previous OpenGL context could not be made current again");
        return false;
    }
    return drawn;
}
