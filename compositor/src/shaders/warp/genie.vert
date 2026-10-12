#version 300 es

// The window poured through a neck into its icon. Every vertex of a COLUMNS x
// ROWS grid is built from gl_VertexID alone. The motion is worked out along
// `along`, the axis pointing from the window to the icon, and across it, so
// one program serves a dock on any edge.

const int COLUMNS = 16;
const int ROWS = 64;
// How far ahead of the far edge the rows nearest the icon travel.
const float LAG = 0.65;
const vec2 CORNERS[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
);

uniform mat3 projection;
// The canvas, output-local physical pixels: x, y, width, height.
uniform vec4 window;
uniform vec2 along;
uniform vec2 across;
// Across extent, then along extent from the far edge to the near one.
uniform vec4 window_span;
uniform vec4 icon_span;
// 1 is the window, 0 the icon.
uniform float progress;

out vec2 v_uv;
out float v_alpha;

float ease(float t) {
    t = clamp(t, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

void main() {
    int quad = gl_VertexID / 6;
    vec2 grid = (vec2(quad % COLUMNS, quad / COLUMNS) + CORNERS[gl_VertexID % 6])
        / vec2(COLUMNS, ROWS);

    float travel = 1.0 - progress;
    float bend = ease(travel / 0.45);
    float slide = ease((travel - 0.15) / 0.85);
    float row = clamp(slide * (1.0 + LAG) - LAG * (1.0 - grid.y), 0.0, 1.0);

    float rest_along = mix(window_span.z, window_span.w, grid.y);
    float rest_across = mix(window_span.x, window_span.y, grid.x);
    float b = mix(rest_along, mix(icon_span.z, icon_span.w, grid.y), row);

    float neck = ease((b - window_span.z) / max(icon_span.z - window_span.z, 1.0)) * bend;
    float a = mix(
        mix(window_span.x, icon_span.x, neck),
        mix(window_span.y, icon_span.y, neck),
        grid.x
    );

    v_alpha = 1.0;
    v_uv = (across * rest_across + along * rest_along - window.xy) / window.zw;
    gl_Position = vec4((projection * vec3(across * a + along * b, 1.0)).xy, 0.0, 1.0);
}
