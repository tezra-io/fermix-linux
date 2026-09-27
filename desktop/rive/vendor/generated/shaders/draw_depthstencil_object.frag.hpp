#pragma once

#include "draw_depthstencil_object.frag.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char draw_depthstencil_object_frag[] = R"===(/*
 * Copyright 2022 Rive
 */

#ifdef EXPORTED_FRAGMENT

// Path draws include draw_path_common.glsl, which declares the textures &
// samplers, so we only need to declare these for image meshes.
#ifdef EXPORTED_DRAW_IMAGE_MESH
FRAG_TEXTURE_BLOCK_BEGIN
TEXTURE_RGBA8(PER_DRAW_BINDINGS_SET, IMAGE_TEXTURE_IDX, EXPORTED_imageTexture);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
DST_COLOR_TEXTURE(EXPORTED_dstColorTexture);
#endif
FRAG_TEXTURE_BLOCK_END

DYNAMIC_SAMPLER_BLOCK_BEGIN
SAMPLER_DYNAMIC_IMAGE(imageSampler)
DYNAMIC_SAMPLER_BLOCK_END
#endif // @DRAW_IMAGE_MESH

FRAG_DATA_MAIN(half4, EXPORTED_drawFragmentMain)
{
#ifdef EXPORTED_DRAW_IMAGE_MESH
    VARYING_UNPACK(v_imageTexCoord, float2);
    VARYING_UNPACK(v_imageModulatedColor, half4);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
    VARYING_UNPACK(v_imageBlendMode, ushort);
#endif
#else
    VARYING_UNPACK(v_paint, float4);
#ifdef EXPORTED_ENABLE_MODULATED_IMAGE
    VARYING_UNPACK(v_image, float3);
#endif
#ifdef EXPORTED_FEATHER_ATLAS_BLIT
    VARYING_UNPACK(v_atlasCoord, float2);
#endif // @FEATHER_ATLAS_BLIT
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
    VARYING_UNPACK(v_blendMode, half);
#endif
#endif // !@DRAW_IMAGE_MESH

#ifdef EXPORTED_DRAW_IMAGE_MESH
    half4 color = TEXTURE_SAMPLE_DYNAMIC_LODBIAS(EXPORTED_imageTexture,
                                                 imageSampler,
                                                 v_imageTexCoord,
                                                 uniforms.mipMapLODBias) *
                  v_imageModulatedColor;
#else
    half coverage =
#ifdef EXPORTED_FEATHER_ATLAS_BLIT
        clamp(TEXTURE_SAMPLE_LOD(EXPORTED_featherAtlasTexture,
                                 featherAtlasSampler,
                                 v_atlasCoord,
                                 .0)
                  .x,
              make_half(.0),
              make_half(1.));
#else
        1.;
#endif

    half4 color = find_paint_color(
#ifdef EXPORTED_ENABLE_MODULATED_IMAGE
        v_image,
#endif
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
        cast_half_to_ushort(v_blendMode),
#endif
        v_paint FRAGMENT_CONTEXT_UNPACK);
#endif

// Need to check both flags here because in GL when KHR_blend_equation_advanced
// is supported, it is possible that neither is defined.
#if defined(EXPORTED_ENABLE_ADVANCED_BLEND) && !defined(EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT)
    // Do the color portion of the blend mode in the shader.
#ifdef EXPORTED_DRAW_IMAGE_MESH
    color.xyz = unmultiply_rgb(color);
    ushort blendMode = v_imageBlendMode;
#else
    ushort blendMode = cast_half_to_ushort(v_blendMode);
#endif
    half4 dstColorPremul = DST_COLOR_FETCH(EXPORTED_dstColorTexture);
    color.xyz =
        advanced_color_blend(color.xyz, dstColorPremul, blendMode) * color.w;
#endif

#ifndef EXPORTED_DRAW_IMAGE_MESH
    color *= coverage;
#endif

    // Certain platforms give us less control of the format of what we are
    // rendering too. Specifically, we are auto converted from linear -> sRGB on
    // render target writes in unreal. In those cases we made need to end up in
    // linear color space
#ifdef EXPORTED_NEEDS_GAMMA_CORRECTION
    if (EXPORTED_NEEDS_GAMMA_CORRECTION)
    {
        color = gamma_to_linear(color);
    }
#endif

    color.xyz = add_dither_if_alpha_nonzero(color.xyz,
                                            color.w,
                                            _fragCoord.xy,
                                            uniforms.ditherScale,
                                            uniforms.ditherBias);

    EMIT_FRAG_DATA(color);
}

#endif // FRAGMENT
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive