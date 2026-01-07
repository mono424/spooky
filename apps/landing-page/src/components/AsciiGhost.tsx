import React, { useState, useEffect } from 'react';

const RAW_GHOST = [
    "   ▄▄████████▄▄   ", // 0
    " ▄██████████████▄ ", // 1
    " ████████████████ ", // 2
    " ████  ████  ████ ", // 3 (Eyes)
    " ████████████████ ", // 4
    " ██████▀  ▀██████ ", // 5
    " ██████    ██████ ", // 6
    " ██████▄  ▄██████ ", // 7
    " ████████████████ ", // 8
    " ████████████████ ", // 9
    " ██▄ ▀█▄▀ █▀ ▄█▄█ ", // 10
    "  ▀   █   █   ▀ ▀ "  // 11
];

const EYES_CLOSED = " ████▀▀████▀▀████ "; // Replacement for row 3

// Helper to pad lines for movement
// Left (-1)   : "trimmed  "
// Center (0)  : " trimmed "
// Right (1)   : "  trimmed"
function shiftLine(line: string, dir: number): string {
    const trimmed = line.trim();
    // Reduce horizontal sway by increasing threshold (now 0.85)
    if (dir < -0.85) return trimmed + "  "; // Left
    if (dir > 0.85) return "  " + trimmed; // Right
    return " " + trimmed + " "; // Center
}

export const AsciiGhost = () => {
    const [phase, setPhase] = useState(0);
    const [blink, setBlink] = useState(false);
    
    // Separate state for smooth float to avoid re-rendering text 60fps
    const [floatY, setFloatY] = useState(0);

    useEffect(() => {
        let startTime = Date.now();
        let animationFrameId: number;

        const animate = () => {
            const now = Date.now();
            const elapsed = now - startTime;

            // Flutter: 3s up, 3s down = 6s period (approx)
            // sin(2 * PI * t / Period)
            // Period = 6000ms
            const p = (elapsed / 6000) * 2 * Math.PI;
            setFloatY(Math.sin(p) * 10); // 10px amplitude matches Flutter

            animationFrameId = requestAnimationFrame(animate);
        };
        
        // Flutter: 100ms interval
        const textInterval = setInterval(() => {
            setPhase(p => p + 0.5); // Flutter: +0.5 increment
            if (Math.random() > 0.95) { // Flutter: > 0.95
                setBlink(true);
                setTimeout(() => setBlink(false), 200);
            }
        }, 100); 

        animate();

        return () => {
            cancelAnimationFrame(animationFrameId);
            clearInterval(textInterval);
        };
    }, []);

    const renderedRows = RAW_GHOST.map((row, index) => {
        const waveValue = Math.sin(index * 0.4 + phase);
        const content = (blink && index === 3) ? EYES_CLOSED : row;
        return shiftLine(content, waveValue);
    });

    return (
        <div className="relative inline-block">
             {/* Background Glow */}
            <div 
                className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-32 h-32 bg-white/20 blur-[50px] rounded-full pointer-events-none"
                style={{ transform: `translate(-50%, calc(-50% + ${floatY}px))` }}
            />
            
            <pre 
                className="relative z-10 text-[6px] sm:text-[8px] md:text-[10px] leading-none font-bold mb-10 whitespace-pre overflow-x-hidden text-center text-white tracking-tighter select-none cursor-default overflow-y-hidden will-change-transform"
                style={{
                    transform: `translateY(${floatY}px)`,
                    textShadow: "0 0 10px rgba(255, 255, 255, 0.5)"
                }}
            >
                {renderedRows.join('\n')}
            </pre>
        </div>
    );
};
