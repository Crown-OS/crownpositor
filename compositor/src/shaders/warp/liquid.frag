#version 300 es

// A drop of glass: the window frosted to its average colours while it is in
// flight, clearing up into the window itself as it arrives, with the icon it
// was lifted from fading over it near the icon. Cut to a rounded shape in the
// quadrilateral's own coordinates, so the corners bend with it.

precision highp float;

// Mip level the frosted window is read from: a blur dozens of pixels wide for
// the cost of one texture fetch.
const float FROST_LOD = 6.0;
// How far the glass is lifted towards white, all over and at its rim.
const float SHEEN = 0.10;
const float RIM_SHEEN = 0.22;
// Width of the rim's light, physical pixels.
const float RIM_WIDTH = 6.0;

uniform sampler2D tex;
uniform sampler2D icon;
uniform float alpha;
uniform vec4 corners_top;
uniform vec4 corners_bottom;
// The window within the canvas: u, v, width, height.
uniform vec4 content;
// Corner radius and edge softness in physical pixels, how much of the
// window shows, how much of the icon.
uniform vec4 shape;

in vec2 v_grid;

out vec4 color;

float rounded_box(in vec2 p, in vec2 b, in float r);

void main() {
    vec2 size = vec2(
        mix(distance(corners_top.xy, corners_top.zw), distance(corners_bottom.xy, corners_bottom.zw), v_grid.y),
        mix(distance(corners_top.xy, corners_bottom.xy), distance(corners_top.zw, corners_bottom.zw), v_grid.x)
    );
    float d = rounded_box((v_grid - 0.5) * size, size * 0.5, shape.x);
    float coverage = clamp(0.5 - d / (fwidth(d) + shape.y), 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }

    float reveal = shape.z;
    vec4 window = textureLod(tex, content.xy + v_grid * content.zw, (1.0 - reveal) * FROST_LOD);
    float rim = 1.0 - smoothstep(0.0, RIM_WIDTH, -d);
    vec3 frost = mix(window.rgb / max(window.a, 1e-3), vec3(1.0), SHEEN + RIM_SHEEN * rim);
    vec4 body = mix(vec4(frost, 1.0), window, reveal);
    vec4 carried = texture(icon, v_grid);

    color = mix(body, carried, shape.w) * (coverage * alpha);
}
