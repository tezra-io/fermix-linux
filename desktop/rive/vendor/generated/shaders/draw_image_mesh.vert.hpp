#pragma once

#include "draw_image_mesh.vert.exports.h"

namespace rive {
namespace gpu {
namespace glsl {
const char draw_image_mesh_vert[] = R"===(/*
 * Copyright 2023 Rive
 */

#ifdef EXPORTED_VERTEX
ATTR_BLOCK_BEGIN(PositionAttr)
ATTR(0, float2, EXPORTED_a_position);
ATTR_BLOCK_END

ATTR_BLOCK_BEGIN(UVAttr)
ATTR(1, float2, EXPORTED_a_texCoord);
ATTR_BLOCK_END

ATTR_BLOCK_BEGIN(ImageDrawAttrs)
ATTR(IMAGE_VIEW_MATRIX_ATTRIB_IDX, float4, EXPORTED_a_imageDrawViewMatrix);
ATTR(IMAGE_CLIP_RECT_INVERSE_MATRIX_ATTRIB_IDX,
     float4,
     EXPORTED_a_imageDrawClipRectInverseMatrix);
ATTR(IMAGE_TRANSLATES_ATTRIB_IDX, float4, EXPORTED_a_imageDrawTranslates);
ATTR(IMAGE_MODULATED_COLOR_ATTRIB_IDX, uint, EXPORTED_a_imageDrawModulatedColor);
ATTR(IMAGE_CLIP_ID_ATTRIB_IDX, uint, EXPORTED_a_imageDrawClipID);
ATTR(IMAGE_BLEND_MODE_ATTRIB_IDX, uint, EXPORTED_a_imageDrawBlendMode);
ATTR(IMAGE_ZINDEX_ATTRIB_IDX, uint, EXPORTED_a_imageDrawZIndex);
ATTR_BLOCK_END
#endif

VARYING_BLOCK_BEGIN
NO_PERSPECTIVE VARYING(0, float2, v_imageTexCoord);
#ifdef EXPORTED_ENABLE_CLIPPING
EXPORTED_OPTIONALLY_FLAT VARYING(1, half, v_clipID);
#endif
#if defined(EXPORTED_ENABLE_CLIP_RECT) && !defined(EXPORTED_RENDER_MODE_DEPTH_STENCIL)
NO_PERSPECTIVE VARYING(2, float4, v_clipRect);
#endif
EXPORTED_OPTIONALLY_FLAT VARYING(3, half4, v_imageModulatedColor);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
FLAT VARYING(4, ushort, v_imageBlendMode);
#endif
VARYING_BLOCK_END

#ifdef EXPORTED_VERTEX
VERTEX_TEXTURE_BLOCK_BEGIN
VERTEX_TEXTURE_BLOCK_END

IMAGE_MESH_VERTEX_MAIN(EXPORTED_drawVertexMain,
                       PositionAttr,
                       position,
                       UVAttr,
                       uv,
                       ImageDrawAttrs,
                       imageDrawAttrs,
                       _vertexID)
{
    ATTR_UNPACK(_vertexID, position, EXPORTED_a_position, float2);
    ATTR_UNPACK(_vertexID, uv, EXPORTED_a_texCoord, float2);
    ATTR_UNPACK(_instanceID, imageDrawAttrs, EXPORTED_a_imageDrawViewMatrix, float4);
    ATTR_UNPACK(_instanceID,
                imageDrawAttrs,
                EXPORTED_a_imageDrawClipRectInverseMatrix,
                float4);
    ATTR_UNPACK(_instanceID, imageDrawAttrs, EXPORTED_a_imageDrawTranslates, float4);
    ATTR_UNPACK(_instanceID, imageDrawAttrs, EXPORTED_a_imageDrawModulatedColor, uint);
    ATTR_UNPACK(_instanceID, imageDrawAttrs, EXPORTED_a_imageDrawClipID, uint);
    ATTR_UNPACK(_instanceID, imageDrawAttrs, EXPORTED_a_imageDrawBlendMode, uint);
    ATTR_UNPACK(_instanceID, imageDrawAttrs, EXPORTED_a_imageDrawZIndex, uint);

    VARYING_INIT(v_imageTexCoord, float2);
#ifdef EXPORTED_ENABLE_CLIPPING
    VARYING_INIT(v_clipID, half);
#endif
#if defined(EXPORTED_ENABLE_CLIP_RECT) && !defined(EXPORTED_RENDER_MODE_DEPTH_STENCIL)
    VARYING_INIT(v_clipRect, float4);
#endif
    VARYING_INIT(v_imageModulatedColor, half4);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
    VARYING_INIT(v_imageBlendMode, ushort);
#endif

    float2 vertexPosition =
        MUL(make_float2x2(EXPORTED_a_imageDrawViewMatrix), EXPORTED_a_position) +
        EXPORTED_a_imageDrawTranslates.xy;
    v_imageTexCoord = EXPORTED_a_texCoord;
#ifdef EXPORTED_ENABLE_CLIPPING
    if (EXPORTED_ENABLE_CLIPPING)
    {
        v_clipID =
            id_bits_to_f16(EXPORTED_a_imageDrawClipID, uniforms.pathIDGranularity);
    }
#endif
#ifdef EXPORTED_ENABLE_CLIP_RECT
    if (EXPORTED_ENABLE_CLIP_RECT)
    {
#ifndef EXPORTED_RENDER_MODE_DEPTH_STENCIL
        v_clipRect = find_clip_rect_coverage_distances(
            make_float2x2(EXPORTED_a_imageDrawClipRectInverseMatrix),
            EXPORTED_a_imageDrawTranslates.zw,
            vertexPosition CLIP_CONTEXT_UNPACK);
#else
        set_clip_rect_plane_distances(
            make_float2x2(EXPORTED_a_imageDrawClipRectInverseMatrix),
            EXPORTED_a_imageDrawTranslates.zw,
            vertexPosition CLIP_CONTEXT_UNPACK);
#endif
    }
#endif // ENABLE_CLIP_RECT
    float4 pos = RENDER_TARGET_COORD_TO_CLIP_COORD(vertexPosition);
#ifdef EXPORTED_POST_INVERT_Y
    pos.y = -pos.y;
#endif
#ifdef EXPORTED_RENDER_MODE_DEPTH_STENCIL
    pos.z = normalize_z_index(EXPORTED_a_imageDrawZIndex);
#endif

    v_imageModulatedColor = unpackUnorm4x8(EXPORTED_a_imageDrawModulatedColor);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
    v_imageBlendMode = cast_uint_to_ushort(EXPORTED_a_imageDrawBlendMode);
#endif

    VARYING_PACK(v_imageTexCoord);
#ifdef EXPORTED_ENABLE_CLIPPING
    VARYING_PACK(v_clipID);
#endif
#if defined(EXPORTED_ENABLE_CLIP_RECT) && !defined(EXPORTED_RENDER_MODE_DEPTH_STENCIL)
    VARYING_PACK(v_clipRect);
#endif
    VARYING_PACK(v_imageModulatedColor);
#ifdef EXPORTED_ENABLE_ADVANCED_BLEND
    VARYING_PACK(v_imageBlendMode);
#endif
    EMIT_VERTEX(pos);
}
#endif
)===";
} // namespace glsl
} // namespace gpu
} // namespace rive