precision mediump float;

varying vec2 uv;

uniform float uTime;
uniform float uAudioLevel;
uniform float uRadius;
uniform float uAspect;
uniform float uAlpha;

// Height of the orb's centre as a fraction of the screen, measured from the
// bottom edge, so the sphere rests low like a physical puck. `voice_orb` in
// `view.rs` must keep its damage rect comfortably larger than the sphere and
// the halo around it.
const float ORB_CENTER_Y = 0.26;

void main() {
    float level = clamp(uAudioLevel, 0.0, 1.0);
    float pulse = 0.5 + 0.5 * sin(uTime * 4.0);
    float radius = uRadius * (1.0 + 0.04 * pulse + 0.10 * level);

    // Centred coordinates; x is scaled so the sphere stays round on 16:9.
    vec2 p = vec2((uv.x - 0.5) * uAspect, uv.y - ORB_CENTER_Y);
    float r = length(p);
    float q = r / radius;

    float disc = 1.0 - smoothstep(radius - 0.0035, radius, r);
    float inside = 1.0 - smoothstep(radius, radius + 0.004, r);
    float halo = exp(-max(r - radius, 0.0) * 24.0) * (0.10 + 0.20 * level);
    float alpha = clamp(max(disc, halo), 0.0, 1.0) * uAlpha;
    if (alpha <= 0.002) {
        gl_FragColor = vec4(0.0);
        return;
    }

    // Shallow dome normal, so the shading reads as a polished sphere.
    float z = sqrt(max(0.0, 1.0 - q * q));
    vec3 n = vec3(p / radius, z);

    vec3 key = normalize(vec3(-0.35, 0.50, 0.78));
    vec3 halfway = normalize(key + vec3(0.0, 0.0, 1.0));
    float diffuse = max(dot(n, key), 0.0);
    float specular = pow(max(dot(n, halfway), 0.0), 48.0);
    // A wide, soft sheen over the upper dome instead of a pin-point highlight.
    float sheen = pow(max(n.y, 0.0), 3.0) * 0.22;
    // Polished silhouettes always catch light; the right edge catches most.
    float rim = pow(1.0 - z, 2.6) * (0.30 + 0.25 * max(n.x, 0.0));
    // The broad studio streak the reference sphere carries across its waist.
    float axis = (n.y - 0.18) * 3.4;
    float streak = exp(-axis * axis) * 0.10;

    // Near-black at the base, graphite where the key light lands.
    vec3 body = mix(vec3(0.020, 0.023, 0.030), vec3(0.36, 0.39, 0.45), pow(diffuse, 1.5));
    vec3 color = body + vec3(sheen + streak + specular * 0.85);
    color += vec3(0.72, 0.80, 1.0) * rim;

    // Iridescent seam: a thin holographic band across the waist that drifts
    // slowly and brightens while the microphone hears the room.
    float wave = 0.075 * sin(n.x * 2.6 + uTime * 0.7) + 0.03 * sin(n.x * 5.1 - uTime * 0.4);
    float offset = n.y - wave;
    float seam = exp(-offset * offset * 260.0) * 0.45 + exp(-offset * offset * 900.0) * 0.85;
    float hue = 0.5 + 0.5 * sin(n.x * 2.2 + 1.9);
    vec3 spectrum = 0.5 + 0.5 * cos(6.28318 * (hue + vec3(0.0, 0.33, 0.67)));
    color += mix(spectrum, vec3(1.0), 0.30) * seam * (0.75 + 1.1 * level) * z;

    // The seam and highlights must not spill past the silhouette.
    color *= inside;
    color += vec3(0.80, 0.88, 1.0) * halo * 0.6;
    // Premultiplied, so the sphere darkens the wallpaper behind it while the
    // seam and highlights still add light.
    color = clamp(color, 0.0, 1.0);
    gl_FragColor = vec4(color * alpha, alpha);
}
