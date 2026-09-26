#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

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

// Two rows of an RGB → Y'CbCr matrix and their offsets, declared on the Rust
// side by `Nv12Shader`. The luma pass sets the first to Y' and writes an R8
// plane, which keeps only red; the chroma pass sets Cb and Cr and writes a
// GR88 plane, whose red and green bytes are NV12's interleaved U and V. The
// chroma plane is half the size of the source in both directions, so linear
// filtering already averages each 2x2 block into the sample at its centre.
uniform vec3 first_row;
uniform float first_offset;
uniform vec3 second_row;
uniform float second_offset;

void main() {
    vec3 rgb = texture2D(tex, v_coords).rgb;
    gl_FragColor = vec4(
        dot(rgb, first_row) + first_offset,
        dot(rgb, second_row) + second_offset,
        0.0,
        1.0
    );
}
