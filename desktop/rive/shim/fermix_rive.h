// The C interface over the Rive runtime that src/lib.rs calls.
//
// A stage is an offscreen EGL context with the Rive GL renderer, and one .riv
// file imported into it. A scene is one artboard of that file, its state
// machine, and the view model instance bound to it, drawn into pixels on
// request. Every call that touches GL makes the stage's context current and
// then puts back whatever context the thread had before, so the caller's own
// GL is left as it was.
//
// A call that can fail writes one sentence into `error` (NUL-terminated and
// cut to `error_len`) and returns NULL or false.

#ifndef FERMIX_RIVE_H
#define FERMIX_RIVE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FxStage FxStage;
typedef struct FxScene FxScene;

FxStage* fx_stage_new(const uint8_t* riv, size_t riv_len, char* error, size_t error_len);
void fx_stage_free(FxStage* stage);

FxScene* fx_scene_new(FxStage* stage, const char* state_machine, char* error, size_t error_len);
void fx_scene_free(FxStage* stage, FxScene* scene);

// Writes an enum property of the scene's view model instance. False when the
// instance has no such property or the enum has no such value.
bool fx_scene_set_enum(FxScene* scene, const char* property, const char* value);
// Writes a number property. False when the instance has no such property.
bool fx_scene_set_number(FxScene* scene, const char* property, float value);
// Advances the state machine and applies it to the artboard.
void fx_scene_advance(FxScene* scene, float seconds);

// Draws the artboard, fitted inside width by height and centred, into `rgba`:
// premultiplied RGBA, 8 bits a channel, top row first, width * 4 bytes a row.
// `rgba_len` must be width * height * 4.
bool fx_scene_render(FxStage* stage,
                     FxScene* scene,
                     uint32_t width,
                     uint32_t height,
                     uint8_t* rgba,
                     size_t rgba_len,
                     char* error,
                     size_t error_len);

#ifdef __cplusplus
}
#endif

#endif
