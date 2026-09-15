// A gaussian-blurred rounded rectangle, drawn from nothing but its own
// geometry.
//
// A *pixel* shader like the border: a drop shadow has no buffer to sample, it
// generates every pixel it draws. Smithay prepends `#version 100` itself,
// substitutes no `//_DEFINES_` line, and compiles only two variants — plain and
// DEBUG_FLAGS.
//
// Blurring a silhouette the way the backdrop is blurred would cost a whole
// pyramid for something that is one flat colour. Instead the distance field is
// pushed through the gaussian's own integral: for a straight edge that is
// exactly right, and a rounded corner is close enough that no eye has ever
// caught the difference.

#if defined(GL_FRAGMENT_PRECISION_HIGH)
precision highp float;
#else
precision mediump float;
#endif

// Supplied by the renderer for every pixel program.
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// Additional uniforms, declared on the Rust side by `ShadowShader`.
// Premultiplied, so scaling every channel by the mask keeps it that way.
uniform vec4 color;
// The silhouette inside this element, in pixels: where it starts, how big it
// is, and how round its corners are. It sits inside the element rather than
// filling it, because the blur needs room to fall off and the offset moves it.
uniform vec2 shape_origin;
uniform vec2 shape_size;
uniform float shape_radius;
// Standard deviation of the gaussian, in pixels.
uniform float sigma;

// Signed distance to a rounded box, after Inigo Quilez.
float rounded_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Abramowitz & Stegun 7.1.26. Six multiplies for the gaussian's integral, which
// is what turns a hard silhouette into a soft one without a single extra tap.
float erf(float x) {
    float s = sign(x);
    x = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * x);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t
                      - 0.284496736) * t + 0.254829592) * t * exp(-x * x);
    return s * y;
}

void main() {
    vec2 half_size = shape_size * 0.5;
    vec2 p = v_coords * size - shape_origin - half_size;

    // A radius wider than the shape would fold the distance field inside out.
    float r = min(shape_radius, min(half_size.x, half_size.y));
    float distance = rounded_box(p, half_size, r);

    // The gaussian's cumulative distribution at the edge: 1 well inside the
    // silhouette, 0.5 on its edge, 0 well outside.
    float coverage = 0.5 - 0.5 * erf(distance / (max(sigma, 0.0001) * 1.4142136));

    vec4 result = color * (alpha * coverage);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
