#pragma once

#include "resolve_atlas.glsl.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char resolve_atlas[] = R"===(/*
 * Copyright 2025 Rive
 */

// This shader provides a mechanism for resolving various atlas types into GL_R8
// so they can be sampled linearly.
//
// Additionally, EXT_shader_pixel_local_storage extension does not have a
// "clear" function, so this shader also provides a clear mechanism for PLS.

#ifdef EXPORTED_VERTEX
VERTEX_MAIN(EXPORTED_atlasResolveVertexMain, Attrs, attrs, _vertexID, _instanceID)
{
    // Draw a right triangle that covers the entire screen.
    float4 pos;
    pos.x = (_vertexID != 2) ? -1. : 3.;
    pos.y = (_vertexID != 1) ? -1. : 3.;
    pos.zw = float2(.0, 1.);
    EMIT_VERTEX(pos);
}
#endif

#ifdef EXPORTED_FRAGMENT

INLINE ivec2 atlas_frag_coord() { return ivec2(floor(gl_FragCoord)); }

#ifdef EXPORTED_ATLAS_RENDER_TARGET_R32UI_FRAMEBUFFER_FETCH

layout(location = 0) inout uint4 coverageCount;
layout(location = 1) out half4 resolvedCoverage;

void main() { resolvedCoverage.x = uintBitsToFloat(coverageCount.x); }

#elif defined(EXPORTED_ATLAS_RENDER_TARGET_R8_PLS_EXT)

#ifdef EXPORTED_CLEAR_COVERAGE
__pixel_local_outEXT PLS { layout(r32f) float coverageCount; };
#else
__pixel_local_inEXT PLS { layout(r32f) float coverageCount; };
layout(location = 0) out half4 resolvedCoverage;
#endif

void main()
{
#ifdef EXPORTED_CLEAR_COVERAGE
    coverageCount = .0;
#else
    resolvedCoverage.x = coverageCount;
#endif
}

#elif defined(EXPORTED_ATLAS_RENDER_TARGET_R32UI_PLS_ANGLE)

layout(binding = 0, r32ui) uniform highp upixelLocalANGLE coverageCount;
layout(location = 0) out half4 resolvedCoverage;

void main()
{
    resolvedCoverage.x = uintBitsToFloat(pixelLocalLoadANGLE(coverageCount).x);
}

#elif defined(EXPORTED_ATLAS_RENDER_TARGET_R32I_ATOMIC_TEXTURE)

layout(binding = 0, r32i) uniform highp coherent iimage2D _atlasImage;
layout(location = 0) out half4 resolvedCoverage;

void main()
{
    resolvedCoverage.x = float(imageLoad(_atlasImage, atlas_frag_coord()).x) *
                         (1. / ATLAS_R32I_FIXED_POINT_FACTOR);
}

#elif defined(EXPORTED_ATLAS_RENDER_TARGET_RGBA8_UNORM)

TEXTURE_RGBA8(PER_FLUSH_BINDINGS_SET, 0, EXPORTED_atlasRenderTexture);
layout(location = 0) out half4 resolvedCoverage;

void main()
{
    // Apply the following weights to the RGBA of each u8x4 coverage value:
    //   - R counts fractional, positive coverage.
    //   - G counts fractional, negative coverage.
    //   - B counts integer, positive coverage.
    //   - A counts integer, negative coverage.
    half4 coverages = TEXEL_FETCH(EXPORTED_atlasRenderTexture, atlas_frag_coord());
    resolvedCoverage.x =
        (coverages.x - coverages.y) * ATLAS_UNORM8_COVERAGE_SCALE_FACTOR +
        (coverages.z - coverages.w) * 255.;
}

#endif

#endif // FRAGMENT
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive