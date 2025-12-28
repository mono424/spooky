import React, { useEffect, useState } from 'react';

export const ArchitectureDiagram = () => {
  const [frame, setFrame] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => {
      setFrame((f) => (f + 1) % 40); // 40 ticks per cycle
    }, 150);
    return () => clearInterval(timer);
  }, []);

  // Helpers to insert character at specific index in a string
  const replaceAt = (str: string, index: number, replacement: string) => {
    return str.substring(0, index) + replacement + str.substring(index + replacement.length);
  };

  const WIDTH = 62;
  // Template with 62 chars width (including newline generally, but we'll use array of strings)
  /*
      [ CLIENT A ]                  [ CLIENT B ]
           |                             |
           |                             |
           v                             v
    +--------------+              +--------------+
    |  LOCAL DB A  |              |  LOCAL DB B  |
    +--------------+              +--------------+
           |                             |
           |                             |
           v                             v
    +--------------------------------------------+
    |             REMOTE SURREALDB               |
    |                                            |
    |  +--------------+      +----------------+  |
    |  | INCANTATIONS |<-----|  DBSP MODULE   |  |
    |  +--------------+      +----------------+  |
    +--------------------------------------------+
  */
  
  const baseLines = [
    "      [ CLIENT A ]                  [ CLIENT B ]      ", // 0
    "           |                             |            ", // 1
    "           |                             |            ", // 2
    "           v                             v            ", // 3
    "    +--------------+              +--------------+    ", // 4
    "    |  LOCAL DB A  |              |  LOCAL DB B  |    ", // 5
    "    +--------------+              +--------------+    ", // 6
    "           |                             |            ", // 7
    "           |                             |            ", // 8
    "           v                             v            ", // 9
    "    +--------------------------------------------+    ", // 10
    "    |             REMOTE SURREALDB               |    ", // 11
    "    |                                            |    ", // 12
    "    |  +--------------+      +----------------+  |    ", // 13
    "    |  | INCANTATIONS |<-----|  DBSP MODULE   |  |    ", // 14
    "    |  +--------------+      +----------------+  |    ", // 15
    "    +--------------------------------------------+    ", // 16
  ];

  // Animation Logic
  const lines = [...baseLines];
  
  // A -> Local A
  if (frame >= 0 && frame < 4) {
      const pos = frame; 
      // rows 1, 2, 3. 
      // frame 0 -> row 1
      // frame 1 -> row 2
      // frame 2 -> row 3
      if (pos < 3) lines[1 + pos] = replaceAt(lines[1 + pos], 11, "o");
  }

  // Flash Local DB A
  if (frame >= 3 && frame < 6) {
      lines[5] = replaceAt(lines[5], 6, " PROCESSING ");
  }

  // Local A -> Remote
  if (frame >= 6 && frame < 9) {
      const pos = frame - 6;
      // rows 7, 8, 9
      if (pos < 3) lines[7 + pos] = replaceAt(lines[7 + pos], 11, "o");
  }

  // Flash Remote (Ingest) -> Internal Move
  if (frame >= 9 && frame < 18) {
     lines[11] = replaceAt(lines[11], 18, "~~ SPOOKY STUFF ~~");
     
     // Internal moving from DBSP (right) to Incantations (left)
     // Row 14, cols 32 to 24 (arrows are at 25-30)
     // DBSP MODULE is at ~36
     // Arrow is |<-----|
     // indices:
     // "    |  | INCANTATIONS |<-----|  DBSP MODULE   |  |    "
     //        ^7             ^23    ^30
     
     // Let's animate the arrow flow right to left
     const arrowStart = 29;
     const arrowEnd = 23;
     const steps = 6;
     const progress = Math.max(0, Math.min(steps, frame - 11));
     
     if (progress < steps) {
        lines[14] = replaceAt(lines[14], arrowStart - progress, "o");
     }
  }

  // Remote -> Local B
  if (frame >= 18 && frame < 22) {
      const pos = frame - 18;
      // Moving UP from Remote to Local B.
      // Rows 9, 8, 7. Col ~41
      // lines[9] aka row index 9.
      const rows = [9, 8, 7, 6]; 
      // Wait, 6 is border.
      if (pos < 3) lines[9 - pos] = replaceAt(lines[9 - pos], 41, "o");
  }

  // Flash Local DB B
  if (frame >= 21 && frame < 24) {
      lines[5] = replaceAt(lines[5], 36, " UPDATING.. ");
  }

  // Local B -> Client B
  if (frame >= 24 && frame < 28) {
      const pos = frame - 24;
      // Rows 3, 2, 1. Col ~41
      if (pos < 3) lines[3 - pos] = replaceAt(lines[3 - pos], 41, "o");
  }
  
  // Flash Client B
  if (frame >= 27 && frame < 32) {
      lines[0] = replaceAt(lines[0], 38, "*UPDATE*");
  }

  return (
    <pre className="text-[10px] sm:text-xs leading-tight font-bold whitespace-pre overflow-x-auto">
      {`+--------------------------------------------------------+
|                  ARCHITECTURE DIAGRAM                  |
+--------------------------------------------------------+
|                                                        |
${lines.map(l => `|${l}  |`).join('\n')}
|                                                        |
+--------------------------------------------------------+`}
    </pre>
  );
};
