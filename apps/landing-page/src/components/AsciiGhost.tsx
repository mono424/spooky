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
    // Use higher threshold (0.7) so it stays centered more often = "less strong" wiggle
    if (dir < -0.7) return trimmed + "  "; // Left
    if (dir > 0.7) return "  " + trimmed; // Right
    return " " + trimmed + " "; // Center
}

export const AsciiGhost = () => {
    const [phase, setPhase] = useState(0);
    const [blink, setBlink] = useState(false);

    useEffect(() => {
        const interval = setInterval(() => {
            // Increment phase for the wave
            setPhase(p => p + 0.5); 
            
            // Random blink
            if (Math.random() > 0.95) {
                setBlink(true);
                setTimeout(() => setBlink(false), 200);
            }
        }, 100); // 10Hz update for smooth animation
        return () => clearInterval(interval);
    }, []);

    const renderedRows = RAW_GHOST.map((row, index) => {
        // Calculate sine wave offset based on row index and time phase
        // Frequency = 0.5 (how tight the wave is)
        // Phase = time offset
        const waveValue = Math.sin(index * 0.6 + phase);
        
        // Use the eyes-closed row if blinking and on index 3
        const content = (blink && index === 3) ? EYES_CLOSED : row;
        
        return shiftLine(content, waveValue);
    });

    return (
        <pre className="text-[8px] sm:text-[10px] md:text-xs leading-none font-bold mb-10 whitespace-pre overflow-x-hidden text-center mx-auto text-white tracking-tighter select-none cursor-default overflow-y-hidden">
            {renderedRows.join('\n')}
        </pre>
    );
};
