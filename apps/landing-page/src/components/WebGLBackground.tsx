import { useEffect, useRef } from 'react';

export default function WebGLBackground() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const mouseRef = useRef({ x: 0.5, y: 0.5 });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext('webgl');
    if (!gl) {
      console.error('WebGL not supported');
      return;
    }

    // Mouse tracking
    const handleMouseMove = (e: MouseEvent) => {
      mouseRef.current = {
        x: e.clientX / window.innerWidth,
        y: 1.0 - (e.clientY / window.innerHeight), // Invert Y for WebGL coordinates
      };
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('touchmove', (e) => {
      if (e.touches.length > 0) {
        mouseRef.current = {
          x: e.touches[0].clientX / window.innerWidth,
          y: 1.0 - (e.touches[0].clientY / window.innerHeight),
        };
      }
    });

    // Resize canvas to fill screen
    const resizeCanvas = () => {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
      gl.viewport(0, 0, canvas.width, canvas.height);
    };
    resizeCanvas();
    window.addEventListener('resize', resizeCanvas);

    // Vertex shader
    const vertexShaderSource = `
      attribute vec2 position;
      void main() {
        gl_Position = vec4(position, 0.0, 1.0);
      }
    `;

    // Fragment shader with animated gradient
    const fragmentShaderSource = `
      precision mediump float;
      uniform float time;
      uniform vec2 resolution;
      uniform vec2 mouse;

      // Color palette based on your theme
      vec3 deepNavy = vec3(0.039, 0.055, 0.102);      // #0a0e1a
      vec3 purple = vec3(0.325, 0.204, 0.514);        // #533483
      vec3 pink = vec3(0.914, 0.271, 0.376);          // #E94560
      vec3 darkBlue = vec3(0.059, 0.204, 0.376);      // #0F3460

      // Simplex noise function
      vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
      vec2 mod289(vec2 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
      vec3 permute(vec3 x) { return mod289(((x*34.0)+1.0)*x); }

      float snoise(vec2 v) {
        const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
        vec2 i  = floor(v + dot(v, C.yy));
        vec2 x0 = v - i + dot(i, C.xx);
        vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
        vec4 x12 = x0.xyxy + C.xxzz;
        x12.xy -= i1;
        i = mod289(i);
        vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
        vec3 m = max(0.5 - vec3(dot(x0,x0), dot(x12.xy,x12.xy), dot(x12.zw,x12.zw)), 0.0);
        m = m*m;
        m = m*m;
        vec3 x = 2.0 * fract(p * C.www) - 1.0;
        vec3 h = abs(x) - 0.5;
        vec3 ox = floor(x + 0.5);
        vec3 a0 = x - ox;
        m *= 1.79284291400159 - 0.85373472095314 * (a0*a0 + h*h);
        vec3 g;
        g.x  = a0.x  * x0.x  + h.x  * x0.y;
        g.yz = a0.yz * x12.xz + h.yz * x12.yw;
        return 130.0 * dot(m, g);
      }

      void main() {
        vec2 uv = gl_FragCoord.xy / resolution;
        vec2 pos = uv * 3.0;

        // Mouse influence
        vec2 mouseOffset = (mouse - uv) * 2.0;
        float mouseDist = length(mouseOffset);
        float mouseInfluence = smoothstep(0.8, 0.0, mouseDist);

        // Multiple layers of noise for complex movement
        float noise1 = snoise(pos + time * 0.05 + mouseOffset * 0.3);
        float noise2 = snoise(pos * 2.0 - time * 0.08 + mouseOffset * 0.2);
        float noise3 = snoise(pos * 0.5 + time * 0.03 + mouseOffset * 0.1);

        // Combine noises
        float combinedNoise = (noise1 + noise2 * 0.5 + noise3 * 0.3) / 1.8;

        // Create flowing pattern with mouse influence
        float pattern = sin(pos.x * 2.0 + time * 0.1 + combinedNoise * 3.0 + mouseInfluence * 2.0) *
                       cos(pos.y * 2.0 - time * 0.15 + combinedNoise * 2.0 + mouseInfluence * 1.5);

        // Add radial gradient from center
        vec2 center = vec2(0.5, 0.5);
        float dist = length(uv - center);
        float radialGradient = 1.0 - smoothstep(0.0, 1.2, dist);

        // Color mixing based on noise and pattern
        vec3 color = deepNavy;

        // Add purple highlights with mouse interaction
        color = mix(color, purple * 0.4, smoothstep(-0.3, 0.3, combinedNoise) * 0.6 * radialGradient);
        color = mix(color, purple * 0.6, mouseInfluence * 0.5);

        // Add pink accents with mouse interaction
        color = mix(color, pink * 0.35, smoothstep(0.4, 0.8, pattern) * 0.5 * radialGradient);
        color = mix(color, pink * 0.5, mouseInfluence * 0.7);

        // Add dark blue waves
        color = mix(color, darkBlue * 0.6, smoothstep(-0.6, -0.2, noise2) * 0.6 * radialGradient);

        // Vignette effect
        float vignette = smoothstep(1.2, 0.3, dist);
        color *= vignette;

        gl_FragColor = vec4(color, 1.0);
      }
    `;

    // Create and compile shaders
    const createShader = (type: number, source: string) => {
      const shader = gl.createShader(type);
      if (!shader) return null;
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        console.error('Shader compile error:', gl.getShaderInfoLog(shader));
        gl.deleteShader(shader);
        return null;
      }
      return shader;
    };

    const vertexShader = createShader(gl.VERTEX_SHADER, vertexShaderSource);
    const fragmentShader = createShader(gl.FRAGMENT_SHADER, fragmentShaderSource);

    if (!vertexShader || !fragmentShader) return;

    // Create program
    const program = gl.createProgram();
    if (!program) return;

    gl.attachShader(program, vertexShader);
    gl.attachShader(program, fragmentShader);
    gl.linkProgram(program);

    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      console.error('Program link error:', gl.getProgramInfoLog(program));
      return;
    }

    gl.useProgram(program);

    // Set up geometry (full screen quad)
    const vertices = new Float32Array([
      -1, -1,
       1, -1,
      -1,  1,
       1,  1,
    ]);

    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);

    const positionLocation = gl.getAttribLocation(program, 'position');
    gl.enableVertexAttribArray(positionLocation);
    gl.vertexAttribPointer(positionLocation, 2, gl.FLOAT, false, 0, 0);

    // Get uniform locations
    const timeLocation = gl.getUniformLocation(program, 'time');
    const resolutionLocation = gl.getUniformLocation(program, 'resolution');
    const mouseLocation = gl.getUniformLocation(program, 'mouse');

    // Animation loop
    let animationFrameId: number;
    const startTime = Date.now();

    const render = () => {
      const time = (Date.now() - startTime) / 1000;

      gl.uniform1f(timeLocation, time);
      gl.uniform2f(resolutionLocation, canvas.width, canvas.height);
      gl.uniform2f(mouseLocation, mouseRef.current.x, mouseRef.current.y);

      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);

      animationFrameId = requestAnimationFrame(render);
    };

    render();

    // Cleanup
    return () => {
      window.removeEventListener('resize', resizeCanvas);
      window.removeEventListener('mousemove', handleMouseMove);
      cancelAnimationFrame(animationFrameId);
      gl.deleteProgram(program);
      gl.deleteShader(vertexShader);
      gl.deleteShader(fragmentShader);
      gl.deleteBuffer(buffer);
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      className="absolute inset-0 w-full h-full"
      style={{ mixBlendMode: 'screen', opacity: 0.55 }}
    />
  );
}
