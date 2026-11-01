#version 300 es

// The window stretched over a quadrilateral: a COLUMNS x ROWS grid built from
// gl_VertexID alone, each vertex placed by interpolating the four corners
// bilinearly, so a trapezoid bends the content smoothly instead of folding it
// along a diagonal the way two flat triangles would.

const int COLUMNS = 16;
const int ROWS = 16;
const vec2 CORNERS[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
);

uniform mat3 projection;
// Top left and top right, then bottom left and bottom right: output-local
// physical pixels.
uniform vec4 corners_top;
uniform vec4 corners_bottom;

out vec2 v_grid;

void main() {
    int quad = gl_VertexID / 6;
    vec2 grid = (vec2(quad % COLUMNS, quad / COLUMNS) + CORNERS[gl_VertexID % 6])
        / vec2(COLUMNS, ROWS);

    vec2 top = mix(corners_top.xy, corners_top.zw, grid.x);
    vec2 bottom = mix(corners_bottom.xy, corners_bottom.zw, grid.x);
    v_grid = grid;
    gl_Position = vec4((projection * vec3(mix(top, bottom, grid.y), 1.0)).xy, 0.0, 1.0);
}
