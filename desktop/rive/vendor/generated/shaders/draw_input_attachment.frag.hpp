#pragma once

#include "draw_input_attachment.frag.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char draw_input_attachment_frag[] = R"===(/*
 * Copyright 2025 Rive
 */

#ifdef EXPORTED_FRAGMENT
layout(input_attachment_index = 0,
#ifdef EXPORTED_INPUT_ATTACHMENT_BINDING
       binding = EXPORTED_INPUT_ATTACHMENT_BINDING,
#else
       binding = 0,
#endif
       set = PLS_TEXTURE_BINDINGS_SET) uniform lowp subpassInput
    inputAttachment;

layout(location = 0) out half4 outputColor;

void main() { outputColor = subpassLoad(inputAttachment); }
#endif
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive