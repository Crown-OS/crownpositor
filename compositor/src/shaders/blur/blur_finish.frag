#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

// dFdx/dFdy for scale-independent edge antialiasing and for the shape's own
// normal — everything that gives the material a direction comes from the
// gradient of the distance field. `enable` so drivers without it fall back to a
// flat sheet of glass rather than failing to compile.
#extension GL_OES_standard_derivatives : enable

#if defined(GL_FRAGMENT_PRECISION_HIGH)
precision highp float;
#else
precision mediump float;
#endif

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// The rounded rectangle the glass is cut from, in the framebuffer pixels
// `gl_FragCoord` is in: one shape, not the piece of it being drawn. A blur
// shape is several primitives, and each piece has to be cut by its own
// primitive rather than by its scissored fragment.
uniform vec2 mask_origin;
uniform vec2 mask_size;
// Corner radius in pixels — must match whatever is drawn on top.
uniform float corner_radius;

// Defined in `shaders/common/glass.glsl`, concatenated after this file and
// after the `rounded_box.glsl` it measures the shape with.
float glass_edge(vec2 p, vec2 size, float radius, out vec2 normal, out float aa);
vec3 glass(vec2 p, float distance, vec2 normal, float aa);

// The pyramid's top level is what this draw places, so the glass reads it as
// the draw's own texture.
vec4 scene_texel(vec2 uv) {
    return texture2D(tex, uv);
}

void main() {
    vec2 p = gl_FragCoord.xy - mask_origin;
    vec2 normal;
    float aa;
    float distance = glass_edge(p, mask_size, corner_radius, normal, aa);
    vec3 color = glass(p, distance, normal, aa);

    // What was copied out of the framebuffer was opaque; make that explicit so
    // stale alpha from the pyramid can never punch a hole in the glass.
    float mask = clamp(0.5 - distance / aa, 0.0, 1.0);
    vec4 result = vec4(color, 1.0) * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
