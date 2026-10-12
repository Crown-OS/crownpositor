#version 300 es

precision highp float;

uniform sampler2D tex;
uniform float alpha;

in vec2 v_uv;
in float v_alpha;

out vec4 color;

void main() {
    color = texture(tex, v_uv) * (alpha * v_alpha);
}
