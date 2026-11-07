#version 300 es

// The canvas laid flat over `window`, built from gl_VertexID alone.

const vec2 CORNERS[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
);

uniform mat3 projection;
// Where the content lands, output-local physical pixels: x, y, width, height.
uniform vec4 window;

out vec2 v_uv;

void main() {
    v_uv = CORNERS[gl_VertexID];
    gl_Position = vec4((projection * vec3(window.xy + v_uv * window.zw, 1.0)).xy, 0.0, 1.0);
}
