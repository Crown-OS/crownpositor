#version 300 es

// A window crumbling into particles that fall away. One quad per cell of the
// canvas, built from gl_VertexID alone, each showing its own piece of the
// window. The bottom gives way first, sweeping up and in from the right, where
// the close control is. A particle released at `delay` then flies ballistically
// under gravity against linear drag, in closed form, so nothing is integrated
// or stored per particle.

const vec2 CORNERS[6] = vec2[6](
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0)
);
const float SMALLEST = 0.35;

uniform mat3 projection;
// The canvas, output-local physical pixels: x, y, width, height.
uniform vec4 window;
uniform ivec2 cells;
uniform uint seed;
// Sweep, jitter, life, elapsed: seconds.
uniform vec4 timing;
// Gravity, drag, sideways spread, upward kick: physical pixels and seconds.
uniform vec4 physics;

out vec2 v_uv;
out float v_alpha;

float random(inout uint state) {
    state = state * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return float((word >> 22u) ^ word) / 4294967295.0;
}

void main() {
    int particle = gl_VertexID / 6;
    vec2 cell = vec2(particle % cells.x, particle / cells.x);
    vec2 corner = CORNERS[gl_VertexID % 6];
    vec2 grid = vec2(cells);
    vec2 size = window.zw / grid;

    uint state = uint(particle) ^ seed;
    float jitter = random(state);
    vec2 launch = vec2(random(state) * 2.0 - 1.0, -random(state));

    vec2 home = (cell + 0.5) / grid;
    float delay = timing.x * (0.65 * (1.0 - home.y) + 0.35 * (1.0 - home.x)) + timing.y * jitter;
    float flight = max(timing.w - delay, 0.0);

    float drag = physics.y;
    vec2 terminal = vec2(0.0, physics.x / drag);
    vec2 initial = launch * physics.zw;
    vec2 travel = terminal * flight + (initial - terminal) * (1.0 - exp(-drag * flight)) / drag;

    float age = clamp(flight / timing.z, 0.0, 1.0);
    float scale = mix(1.0, SMALLEST, age);
    v_alpha = 1.0 - smoothstep(0.35, 1.0, age);
    v_uv = (cell + corner) / grid;

    vec2 position = window.xy + home * window.zw + travel + (corner - 0.5) * size * scale;
    gl_Position = vec4((projection * vec3(position, 1.0)).xy, 0.0, 1.0);
}
