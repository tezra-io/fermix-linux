#pragma once

#include "init_clockwise_atomic_workaround.frag.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char init_clockwise_atomic_workaround_frag[] = R"===(/*
 * Copyright 2026 Rive
 */

// This shader implements a seeming workaround for Qualcomm. Basically, input
// attachment reads of the clip and color buffers don't work unless we first
// draw these buffers into themselves between borrowed coverage and the main
// subpass. This draw is issued with a scissor that only allows one pixel
// through, so the fill rate impact should be negligible.
#ifdef EXPORTED_FRAGMENT

PLS_BLOCK_BEGIN
#ifndef EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT
PLS_DECL4F(COLOR_PLANE_IDX, colorBuffer);
#endif
PLS_DECL4F(CLIP_PLANE_IDX, clipBuffer);
PLS_BLOCK_END

CLOCKWISE_ATOMIC_PLS_MAIN(EXPORTED_drawFragmentMain)
{
    // Draw the clip buffer onto itself.
    PLS_STORE4F(clipBuffer, make_half4(PLS_LOAD4F(clipBuffer).x, .0, .0, 1.));
#ifndef EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT
    // Draw the color buffer onto itself.
    EMIT_CLOCKWISE_ATOMIC_PLS(PLS_LOAD4F(colorBuffer));
#else
    // This render pass doesn't read the color buffer. Emit 0 (since srcOver
    // blend is enabled) to leave the color buffer unaffected.
    EMIT_CLOCKWISE_ATOMIC_PLS(make_half4(.0));
#endif
}

#endif
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive