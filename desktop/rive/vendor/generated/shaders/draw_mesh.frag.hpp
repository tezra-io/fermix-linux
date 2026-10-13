#pragma once

#include "draw_mesh.frag.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char draw_mesh_frag[] = R"===(/*
 * Copyright 2023 Rive
 */

#ifdef EXPORTED_FRAGMENT

// This is a basic fragment shader for non-msaa, non-path objects, e.g., image
// meshes, atlas blits.
// These objects are simple in that they can write their fragments out directly,
// without having to cooperate with overlapping fragments to work out coverage.

#if (defined(EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT) && !defined(EXPORTED_ENABLE_CLIPPING)) ||   \
    defined(EXPORTED_RENDER_MODE_CLOCKWISE_ATOMIC)
// @FIXED_FUNCTION_COLOR_OUTPUT without clipping can skip the interlock.
#undef NEEDS_INTERLOCK
#else
#define NEEDS_INTERLOCK
#endif

PLS_BLOCK_BEGIN
#ifndef EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT
PLS_DECL4F(COLOR_PLANE_IDX, colorBuffer);
#endif
#ifndef EXPORTED_RENDER_MODE_CLOCKWISE_ATOMIC
PLS_DECLUI(CLIP_PLANE_IDX, clipBuffer);
#ifndef EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT
PLS_DECL4F(SCRATCH_COLOR_PLANE_IDX, scratchColorBuffer);
#endif
PLS_DECLUI(COVERAGE_PLANE_IDX, coverageBuffer);
#else // @RENDER_MODE_CLOCKWISE_ATOMIC
PLS_DECL4F(CLIP_PLANE_IDX, clipBuffer);
#endif
PLS_BLOCK_END

// FEATHER_ATLAS_BLIT includes draw_path_common.glsl, which declares the
// textures & samplers, so we only need to declare these for image meshes.
#ifdef EXPORTED_DRAW_IMAGE_MESH
FRAG_TEXTURE_BLOCK_BEGIN
TEXTURE_RGBA8(PER_DRAW_BINDINGS_SET, IMAGE_TEXTURE_IDX, EXPORTED_imageTexture);
FRAG_TEXTURE_BLOCK_END

DYNAMIC_SAMPLER_BLOCK_BEGIN
SAMPLER_DYNAMIC_IMAGE(imageSampler)
DYNAMIC_SAMPLER_BLOCK_END

FRAG_STORAGE_BUFFER_BLOCK_BEGIN
FRAG_STORAGE_BUFFER_BLOCK_END
#endif // @DRAW_IMAGE_MESH

#ifdef EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT
#ifdef EXPORTED_DRAW_IMAGE_MESH
PLS_FRAG_COLOR_MAIN(EXPORTED_drawFragmentMain)
#else
PLS_FRAG_COLOR_MAIN(EXPORTED_drawFragmentMain)
#endif
#else
#ifdef EXPORTED_DRAW_IMAGE_MESH
PLS_MAIN(EXPORTED_drawFragmentMain)
#else
PLS_MAIN(EXPORTED_drawFragmentMain)
#endif
#endif
{
#ifdef EXPORTED_FEATHER_ATLAS_BLIT
    VARYING_UNPACK(v_paint, float4);
#if defined(EXPORTED_ENABLE_MODULATED_IMAGE)
    VARYING_UNPACK(v_image, float3);
#endif
    VARYING_UNPACK(v_atlasCoord, float2);
#endif
#ifdef EXPORTED_ENABLE_CLIPPING
    VARYING_UNPACK(v_clipID, half);
#endif
#ifdef EXPORTED_ENABLE_CLIP_RECT
    VARYING_UNPACK(v_clipRect, float4);
#endif
#if defined(EXPORTED_FEATHER_ATLAS_BLIT) && defined(EXPORTED_ENABLE_ADVANCED_BLEND)
    VARYING_UNPACK(v_blendMode, half);
#endif
#ifdef EXPORTED_DRAW_IMAGE_MESH
    VARYING_UNPACK(v_imageTexCoord, float2);
    VARYING_UNPACK(v_imageModulatedColor, half4);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
    VARYING_UNPACK(v_imageBlendMode, ushort);
#endif
#endif

#ifdef EXPORTED_FEATHER_ATLAS_BLIT
    half4 color = find_paint_color(
#ifdef EXPORTED_ENABLE_MODULATED_IMAGE
        v_image,
#endif
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
        cast_half_to_ushort(v_blendMode),
#endif
        v_paint FRAGMENT_CONTEXT_UNPACK);
    half coverage = clamp(TEXTURE_SAMPLE_LOD(EXPORTED_featherAtlasTexture,
                                             featherAtlasSampler,
                                             v_atlasCoord,
                                             .0)
                              .x,
                          make_half(.0),
                          make_half(1.));
#endif

#ifdef EXPORTED_DRAW_IMAGE_MESH
    half4 color = TEXTURE_SAMPLE_DYNAMIC_LODBIAS(EXPORTED_imageTexture,
                                                 imageSampler,
                                                 v_imageTexCoord,
                                                 uniforms.mipMapLODBias);
    half coverage = 1.;
#endif

#ifdef EXPORTED_ENABLE_CLIP_RECT
    // Calculate the clip rect before entering the interlock.
    if (EXPORTED_ENABLE_CLIP_RECT)
    {
        half clipRectCoverage =
            max(min_component(cast_float4_to_half4(v_clipRect)), make_half(.0));
        coverage = min(clipRectCoverage, coverage);
    }
#endif

#ifdef NEEDS_INTERLOCK
    PLS_INTERLOCK_BEGIN;
#endif

#if defined(EXPORTED_ENABLE_CLIPPING)
    if (EXPORTED_ENABLE_CLIPPING && v_clipID != .0)
    {
        half clipCoverage;
#ifndef EXPORTED_RENDER_MODE_CLOCKWISE_ATOMIC
        half2 clipData = unpackHalf2x16(PLS_LOADUI(clipBuffer));
        half clipContentID = clipData.y;
        clipCoverage =
            max(clipContentID == v_clipID ? clipData.x : make_half(.0),
                make_half(.0));
#else
        clipCoverage = PLS_LOAD4F(clipBuffer).x;
#endif
        clipCoverage = max(clipCoverage, make_half(.0));
        coverage = min(coverage, clipCoverage);
    }
#endif

#ifdef EXPORTED_DRAW_IMAGE_MESH
    color *= v_imageModulatedColor;
#endif

#if !defined(EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT)
    half4 dstColorPremul = PLS_LOAD4F(colorBuffer);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
#ifdef EXPORTED_FEATHER_ATLAS_BLIT
    ushort blendMode = cast_half_to_ushort(v_blendMode);
#endif
#ifdef EXPORTED_DRAW_IMAGE_MESH
    ushort blendMode = v_imageBlendMode;
#endif
    if (EXPORTED_ENABLE_ADVANCED_BLEND && blendMode != BLEND_SRC_OVER)
    {
        // Advanced-blend draws operate on unmultiplied color.
#ifdef EXPORTED_DRAW_IMAGE_MESH
        color.xyz = unmultiply_rgb(color);
#endif
        color.xyz = advanced_color_blend(color.xyz, dstColorPremul, blendMode) *
                    color.w;
    }
#endif // @ENABLE_ADVANCED_BLEND
    color *= coverage;

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

#ifndef EXPORTED_RENDER_MODE_CLOCKWISE_ATOMIC
    color = dstColorPremul * (1. - color.w) + color;
#endif

    PLS_STORE4F(colorBuffer, color);
#endif // !@FIXED_FUNCTION_COLOR_OUTPUT

#ifndef EXPORTED_RENDER_MODE_CLOCKWISE_ATOMIC
    PLS_PRESERVE_UI(clipBuffer);
    PLS_PRESERVE_UI(coverageBuffer);
#else
    // Since blend is enabled, storing 0 to the clip will ensure it remains
    // unchanged.
    PLS_STORE4F(clipBuffer, make_half4(.0));
#endif
#ifdef NEEDS_INTERLOCK
    PLS_INTERLOCK_END;
#endif

#ifdef EXPORTED_FIXED_FUNCTION_COLOR_OUTPUT
    color = (color * coverage);
    color.xyz = add_dither_if_alpha_nonzero(color.xyz,
                                            color.w,
                                            _fragCoord.xy,
                                            uniforms.ditherScale,
                                            uniforms.ditherBias);
    _fragColor = color;
    EMIT_PLS_AND_FRAG_COLOR
#else
    EMIT_PLS;
#endif
}

#endif // @FRAGMENT
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive