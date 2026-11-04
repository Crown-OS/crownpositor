#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

// dFdx/dFdy and fwidth() for the edges, the glass's normal and the area
// filter's texel size. `enable` so drivers without it fall back to a fixed 1px
// edge, a flat sheet of glass and plain bilinear sampling.
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

// A surface laid on its own glass and cut with it once (`composited_blur`).
//
// Drawn as two layers — the glass cut by the shape's edge, then the surface's
// own antialiased body over it — an edge pixel is covered by each of them, and
// comes out as body, glass and what lies behind in the wrong proportions. Here
// the surface's pixels are composited over the glass first and only the result
// is cut by the edge, so the client paints its body square to the shape's
// rectangle and the edge is the shape's alone.

// The clip the surface is drawn in, exactly as `rounded_corner.frag` takes it:
// the window in framebuffer pixels, its radii per framebuffer corner in the
// order `rounded_box` reads them, and the area filter's buffer pixels per
// framebuffer pixel, or zero to sample the texture as it is.
uniform vec2 shape_origin;
uniform vec2 shape_size;
uniform vec4 radius;
uniform vec2 resample;
// The rounded rectangle the glass is cut from, in framebuffer pixels. Inside
// that rectangle the surface lies on the glass and is cut with it; outside it
// the surface is drawn as it is.
uniform vec2 mask_origin;
uniform vec2 mask_size;
uniform float corner_radius;
// The blurred scene, on a texture unit of its own beside the surface's.
// `glazed` is 1.0 when it was blurred for this draw, and 0.0 when there is no
// glass to lay the surface on — no blur on this frame, or none at all — and the
// surface is only cut.
uniform sampler2D scene_tex;
uniform float glazed;

// Defined in `shaders/common/rounded_box.glsl` and `shaders/common/glass.glsl`,
// concatenated after this file in that order.
float rounded_box(in vec2 p, in vec2 b, in vec4 r);
float glass_edge(vec2 p, vec2 size, float radius, out vec2 normal, out float aa);
vec3 glass(vec2 p, float distance, vec2 normal, float aa);

vec4 scene_texel(vec2 uv) {
    return texture2D(scene_tex, uv);
}

#if defined(GL_OES_standard_derivatives)
// The area filter `rounded_corner.frag` samples every client surface with, so
// a surface looks the same on its glass as anywhere else. One axis of it: an
// output pixel `width` texels wide centred on `centre` covers up to three
// texels; each is weighted by how much of it the pixel covers, and the three
// are folded into two bilinear taps — the first two blended in proportion, the
// third on its own.
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

// The buffer as the output pixel at `coords` should show it, averaged over
// exactly the texels the pixel covers when the buffer is drawn at another size
// than its own.
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

    // The window's own clip, as `rounded_corner.frag` cuts it.
    vec2 half_size = shape_size * 0.5;
    vec4 r = min(radius, vec4(min(half_size.x, half_size.y)));
    float clip_distance = rounded_box(gl_FragCoord.xy - shape_origin - half_size, half_size, r);
#if defined(GL_OES_standard_derivatives)
    float clip_aa = max(length(vec2(dFdx(clip_distance), dFdy(clip_distance))), 0.0001);
#else
    float clip_aa = 1.0;
#endif
    float clip = clamp(0.5 - clip_distance / clip_aa, 0.0, 1.0);

    // The glass's edge, measured for every fragment: its derivatives are taken
    // across the 2x2 quad, which a branch would split.
    vec2 p = gl_FragCoord.xy - mask_origin;
    vec2 normal;
    float aa;
    float distance = glass_edge(p, mask_size, corner_radius, normal, aa);

    // `gl_FragCoord` sits on pixel centres and the rectangle on whole pixels,
    // so a fragment is cleanly inside or outside it.
    bool inside = all(greaterThanEqual(p, vec2(0.0))) && all(lessThan(p, mask_size));
    if (inside) {
        // One cut for the surface and its glass together. Where the window's
        // clip crosses the shape, the nearer edge is the one that cuts.
        float cut = min(clip, clamp(0.5 - distance / aa, 0.0, 1.0));
        vec4 under = vec4(0.0);
        if (glazed > 0.0 && cut > 0.0) {
            under = vec4(glass(p, distance, normal, aa), 1.0);
        }
        // The texture is premultiplied, so this is the surface over the glass.
        color = (color + (1.0 - color.a) * under) * cut;
    } else {
        color *= clip;
    }
    color *= alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
