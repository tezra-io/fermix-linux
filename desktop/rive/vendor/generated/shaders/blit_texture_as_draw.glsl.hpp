#pragma once

#include "blit_texture_as_draw.glsl.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char blit_texture_as_draw[] = R"===(/*
 * Copyright 2024 Rive
 */

VARYING_BLOCK_BEGIN
#ifdef EXPORTED_USE_FILTERING
NO_PERSPECTIVE VARYING(0, float2, v_texCoord);
#endif
VARYING_BLOCK_END

#ifdef EXPORTED_VERTEX
VERTEX_TEXTURE_BLOCK_BEGIN
VERTEX_TEXTURE_BLOCK_END

VERTEX_STORAGE_BUFFER_BLOCK_BEGIN
VERTEX_STORAGE_BUFFER_BLOCK_END

ATTR_BLOCK_BEGIN(Attrs)
ATTR_BLOCK_END

VERTEX_MAIN(EXPORTED_blitVertexMain, Attrs, attrs, _vertexID, _instanceID)
{
    // Fill the entire screen. The caller will use a scissor test to control the
    // bounds being drawn.
    float2 coord;
    coord.x = (_vertexID & 1) == 0 ? -1. : 1.;
    coord.y = (_vertexID & 2) == 0 ? -1. : 1.;
#ifdef EXPORTED_USE_FILTERING
    VARYING_INIT(v_texCoord, float2);
    v_texCoord.x = coord.x * .5 + .5;
    v_texCoord.y = coord.y * -.5 + .5;
    VARYING_PACK(v_texCoord);
#endif
    float4 pos = float4(coord, 0, 1);
    EMIT_VERTEX(pos);
}
#endif // @VERTEX

#ifdef EXPORTED_FRAGMENT
FRAG_TEXTURE_BLOCK_BEGIN
#ifdef EXPORTED_SOURCE_TEXTURE_MSAA
TEXTURE_RGBA8_MS(PER_DRAW_BINDINGS_SET, IMAGE_TEXTURE_IDX, EXPORTED_sourceTexture);
#else
TEXTURE_RGBA8(PER_DRAW_BINDINGS_SET, IMAGE_TEXTURE_IDX, EXPORTED_sourceTexture);
#endif
FRAG_TEXTURE_BLOCK_END

#ifdef EXPORTED_USE_FILTERING
DYNAMIC_SAMPLER_BLOCK_BEGIN
SAMPLER_DYNAMIC_IMAGE(blitSampler)
DYNAMIC_SAMPLER_BLOCK_END
#endif

FRAG_DATA_MAIN(half4, EXPORTED_blitFragmentMain)
{
    half4 srcColor;
#ifdef EXPORTED_USE_FILTERING
    VARYING_UNPACK(v_texCoord, float2);
    srcColor =
        TEXTURE_SAMPLE_DYNAMIC_LOD(EXPORTED_sourceTexture, blitSampler, v_texCoord, .0);
#elif defined(EXPORTED_SOURCE_TEXTURE_MSAA)
    srcColor = (TEXEL_FETCH_MS(EXPORTED_sourceTexture, 0, int2(floor(_fragCoord.xy))) +
                TEXEL_FETCH_MS(EXPORTED_sourceTexture, 1, int2(floor(_fragCoord.xy))) +
                TEXEL_FETCH_MS(EXPORTED_sourceTexture, 2, int2(floor(_fragCoord.xy))) +
                TEXEL_FETCH_MS(EXPORTED_sourceTexture, 3, int2(floor(_fragCoord.xy)))) *
               0.25;
#else
    srcColor = TEXEL_FETCH(EXPORTED_sourceTexture, int2(floor(_fragCoord.xy)));
#endif
    EMIT_FRAG_DATA(srcColor);
}
#endif // @FRAGMENT
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive