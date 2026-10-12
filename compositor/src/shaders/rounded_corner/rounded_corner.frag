#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

// Gives us fwidth(), for scale-independent edge antialiasing and the area
// filter's texel size. `enable` rather than `require` so drivers without it
// fall back to the fixed 1px width and plain bilinear sampling below.
#extension GL_OES_standard_derivatives : enable

// The signed distance math needs more headroom than mediump gives us for large
// outputs, so ask for highp where the implementation supports it.
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

// Additional uniforms, declared on the Rust side by `RoundedCornerShader`.
// The shape the element is clipped to — the window, not this one surface of
// it — in framebuffer pixels, the space `gl_FragCoord` is in. Measuring there
// rather than from `v_coords` is what keeps the corners on the window when a
// buffer is cropped (a client's own shadow margin, a resize in flight) or a
// subsurface covers only part of it: `v_coords` is texture UV, not position.
uniform vec2 shape_origin;
uniform vec2 shape_size;
// Corner radii in pixels for the (+x, +y), (+x, -y), (-x, +y) and (-x, -y)
// framebuffer corners — the order `rounded_box` reads them in. A decorated
// window squares the pair under its titlebar, which rounds those instead.
uniform vec4 radius;
// Buffer pixels per framebuffer pixel along the texture's own axes, or zero to
// sample the texture as it is: drawn 1:1, or so far from it that no area
// filter of three texels a side could cover an output pixel.
uniform vec2 resample;

// Defined in `shaders/common/rounded_box.glsl`, concatenated after this file
// so the colour-managed variant of this shader can share the one copy.
float rounded_box(in vec2 p, in vec2 b, in vec4 r);

#if defined(GL_OES_standard_derivatives)
// One axis of an area filter. An output pixel `width` texels wide centred on
// `centre` covers up to three texels; each is weighted by how much of it the
// pixel covers, and the three are folded into two bilinear taps — the first
// two blended in proportion, the third on its own.
void area_taps(float centre, float width, out vec2 taps, out vec2 weights) {
    float start = centre - 0.5 * width;
    float end = centre + 0.5 * width;
    float first = floor(start);
    float w0 = min(first + 1.0, end) - start;
    float w1 = clamp(end - first - 1.0, 0.0, 1.0);
    float w2 = max(end - first - 2.0, 0.0);
    taps = vec2(first + 0.5 + w1 / (w0 + w1), first + 2.5);
    weights = vec2(w0 + w1, w2) / width;
}
#endif

// The buffer as the output pixel at `coords` should show it. A buffer drawn
// at another size than its own — a client that rendered at 2x on a 1.25x
// output — is averaged over exactly the texels each output pixel covers:
// bilinear filtering blends only the nearest two, so it alternately blurs and
// drops thin stems, and text comes out uneven.
vec4 sample_texture(vec2 coords) {
#if defined(GL_OES_standard_derivatives)
    vec2 footprint = fwidth(coords);
    if (resample.x > 0.0) {
        vec2 texel = footprint / resample;
        vec2 centre = coords / texel;
        vec2 x_taps;
        vec2 x_weights;
        vec2 y_taps;
        vec2 y_weights;
        area_taps(centre.x, resample.x, x_taps, x_weights);
        area_taps(centre.y, resample.y, y_taps, y_weights);
        return texture2D(tex, vec2(x_taps.x, y_taps.x) * texel) * (x_weights.x * y_weights.x)
            + texture2D(tex, vec2(x_taps.y, y_taps.x) * texel) * (x_weights.y * y_weights.x)
            + texture2D(tex, vec2(x_taps.x, y_taps.y) * texel) * (x_weights.x * y_weights.y)
            + texture2D(tex, vec2(x_taps.y, y_taps.y) * texel) * (x_weights.y * y_weights.y);
    }
#endif
    return texture2D(tex, coords);
}

void main() {
    vec4 color = sample_texture(v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    vec2 half_size = shape_size * 0.5;
    // A radius wider than the shape would fold the distance field inside out.
    float limit = min(half_size.x, half_size.y);
    vec4 r = min(radius, vec4(limit));
    float distance = rounded_box(gl_FragCoord.xy - shape_origin - half_size, half_size, r);

#if defined(GL_OES_standard_derivatives)
    // Width of one screen pixel in distance-field units, so the edge stays one
    // pixel wide however the element is scaled.
    float aa = max(fwidth(distance), 0.0001);
#else
    float aa = 1.0;
#endif
    float mask = 1.0 - smoothstep(-0.5 * aa, 0.5 * aa, distance);

    // The texture is premultiplied, so scaling every channel keeps it that way.
    color = color * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
