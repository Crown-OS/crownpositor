#version 100

// Replaced by the renderer with the `#define`s for the variant being compiled:
// EXTERNAL, NO_ALPHA and DEBUG_FLAGS. Must stay on a line of its own.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

// fwidth() for scale-independent edge antialiasing, and dFdx/dFdy for the
// shape's own normal — everything below that gives the material a direction
// comes from the gradient of the distance field. `enable` so drivers without it
// fall back to a flat sheet of glass rather than failing to compile.
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

// The blurred scene is output-wide and shared by every piece of glass on the
// frame, so a fragment addresses it by `gl_FragCoord` alone: no per-backdrop
// origin, and the taps at the edge of one piece land on the real pixels next to
// it rather than on a clamped copy of its own border. A rotated or flipped
// output needs no special case either — the framebuffer is the one space every
// position below agrees on.
uniform vec2 scene_size;
// The rounded rectangle the glass is cut from, in those same framebuffer
// pixels: one shape, not the piece of it being drawn. A blur shape is several
// primitives, and each piece has to be cut by its own primitive rather than by
// its scissored fragment.
uniform vec2 mask_origin;
uniform vec2 mask_size;
// Half a pixel of the pyramid's top level in UV space, and the kawase spread:
// this pass *is* the last upsample, straight into the frame.
uniform vec2 half_pixel;
uniform float offset;
// Corner radius in pixels — must match whatever is drawn on top.
uniform float corner_radius;
// Dither strength; hides the banding a strong blur produces on gradients.
uniform float noise;
// Straight RGBA mixed over the blurred backdrop.
uniform vec4 glass_tint;
// Chroma multiplier about the luma. Above 1.0 is the vibrancy that makes a
// colour show through frosted glass instead of washing out into grey.
uniform float saturation;
// Width of the refractive rim just inside the shape's edge, in pixels. Zero
// leaves the edge flat.
uniform float rim;
// The direction "up and to the left of the screen" points in, in framebuffer
// pixels. Supplied rather than assumed, because the framebuffer's axes are the
// output transform's, not the screen's.
uniform vec2 light;

// Rec. 709 luma, the axis vibrancy rotates the colour about.
const vec3 LUMA = vec3(0.2126, 0.7152, 0.0722);
// How much more saturated the refracted rim is than the glass behind it. Light
// bent through the bevel takes a longer path through the material, so it comes
// back with more of the material's colour in it.
const float RIM_VIBRANCY = 0.45;
// Depth of the inner shadow under the bevel, and brightness of the specular
// line on it.
const float INNER_SHADOW = 0.28;
const float SPECULAR = 0.30;
// What the two faces the light does *not* strike still get, so the rim reads as
// a continuous edge rather than two arcs.
const float AMBIENT = 0.35;

vec2 cl(vec2 uv) {
    return clamp(uv, half_pixel, vec2(1.0) - half_pixel);
}

// Signed distance to a rounded box, after Inigo Quilez. Negative inside.
float rounded_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Cheap screen-space hash (Dave Hoskins) for the dither.
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Dual-kawase upsample: 8 taps, diagonals weighted 2, edges 1. The last level
// of the pyramid, sampled around whatever centre the refraction chose.
vec3 upsample(vec2 uv) {
    vec2 o = half_pixel * offset;
    vec4 sum = vec4(0.0);
    sum += texture2D(tex, cl(uv + vec2(-o.x * 2.0, 0.0)));
    sum += texture2D(tex, cl(uv + vec2( o.x * 2.0, 0.0)));
    sum += texture2D(tex, cl(uv + vec2(0.0, -o.y * 2.0)));
    sum += texture2D(tex, cl(uv + vec2(0.0,  o.y * 2.0)));
    sum += texture2D(tex, cl(uv + vec2(-o.x,  o.y))) * 2.0;
    sum += texture2D(tex, cl(uv + vec2( o.x,  o.y))) * 2.0;
    sum += texture2D(tex, cl(uv + vec2(-o.x, -o.y))) * 2.0;
    sum += texture2D(tex, cl(uv + vec2( o.x, -o.y))) * 2.0;
    return (sum / 12.0).rgb;
}

void main() {
    // The shape, in its own space. Everything the material does to itself is
    // measured here, so a shape split into several drawn pieces still lights as
    // one object.
    vec2 p = gl_FragCoord.xy - mask_origin;
    vec2 half_size = mask_size * 0.5;
    float r = min(corner_radius, min(half_size.x, half_size.y));
    float distance = rounded_box(p - half_size, half_size, r);

#if defined(GL_OES_standard_derivatives)
    // The outward normal, straight out of the field: a signed distance function
    // has unit gradient, so this is the real surface normal of whatever shape
    // the mask describes, with no per-shape case analysis.
    vec2 gradient = vec2(dFdx(distance), dFdy(distance));
    float slope = length(gradient);
    vec2 normal = slope > 0.0 ? gradient / slope : vec2(0.0);
    float aa = max(fwidth(distance), 0.0001);
#else
    vec2 normal = vec2(0.0);
    float aa = 1.0;
#endif

    // How far into the bevel this fragment is: 1 at the very edge, falling off
    // over `rim` pixels inward. The exponential is what makes the edge read as
    // a curve rather than a chamfer.
    float bevel = rim > 0.0 ? exp(-max(-distance, 0.0) / rim) : 0.0;
    // A thinner, brighter band inside the same bevel — the specular line.
    float highlight = bevel * bevel;
    // The two faces the light strikes: top-left and bottom-right. `abs` is what
    // puts it on both, and the ambient floor keeps the other two from going
    // dead flat.
    float facing = mix(AMBIENT, 1.0, abs(dot(normal, light)));

    // Refraction. The bevel bends what is behind it, so the tap centre walks
    // along the normal in proportion to how deep into the curve we are — which
    // costs nothing, because it moves the taps the upsample was going to make
    // anyway.
    vec2 refracted = gl_FragCoord.xy - normal * (bevel * rim);
    vec3 color = upsample(refracted / scene_size);

    // Vibrancy, about the luma so the blur keeps its brightness. The rim gets
    // more of it than the sheet: light through the bevel travels further
    // through the material.
    float luma = dot(color, LUMA);
    color = max(mix(vec3(luma), color, saturation * (1.0 + RIM_VIBRANCY * bevel)), 0.0);
    color = mix(color, glass_tint.rgb, glass_tint.a);

    // The bevel's own shading: a soft inner shadow under it, and a specular
    // line on it. The shadow is held back where the highlight is, so the two do
    // not fight over the same pixels.
    color *= 1.0 - INNER_SHADOW * facing * bevel * (1.0 - highlight);
    color += SPECULAR * facing * highlight;

    if (noise > 0.0) {
        color += vec3((hash(p) - 0.5) * noise);
    }

    // What was copied out of the framebuffer was opaque; make that explicit so
    // stale alpha from the pyramid can never punch a hole in the glass.
    float mask = 1.0 - smoothstep(-0.5 * aa, 0.5 * aa, distance);
    vec4 result = vec4(color, 1.0) * (alpha * mask);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        result = vec4(0.0, 0.2, 0.0, 0.2) + result * 0.8;
#endif

    gl_FragColor = result;
}
