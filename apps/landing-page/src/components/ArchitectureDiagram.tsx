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
    "      [   APP A  ]                  [   APP B  ]      ", // 0
    "           ▲                             ▲            ", // 1
    "           │                             │            ", // 2
    "           ▼                             ▼            ", // 3
    "    +--------------+              +--------------+    ", // 4
    "    |  LOCAL DB A  |              |  LOCAL DB B  |    ", // 5
    "    +--------------+              +--------------+    ", // 6
    "           ▲                             ▲            ", // 7
    "           │                             │            ", // 8
    "           ▼                             ▼            ", // 9
    "    +--------------------------------------------+    ", // 10
    "    |             REMOTE SURREALDB               |    ", // 11
    "    |                                            |    ", // 12
    "    |  +----------+            +--------------+  |    ", // 13
    "    |  |  EVENTS  |            | INCANTATIONS |  |    ", // 14
    "    |  +----------+            +--------------+  |    ", // 15
    "    |       │                          ▲         |    ", // 16 (DEEP ARROWS)
    "    +-------│--------------------------│---------+    ", // 17
    "            │                          │              ", // 18
    "            ▼                          │              ", // 19
    "    +--------------------------------------------+    ", // 20
    "    |           SPOOKY STREAM PROCESSOR          |    ", // 21
    "    +--------------------------------------------+    ", // 22
  ];

  // Animation Logic
  const lines = [...baseLines];

  // Animation State
  const activeModules = new Set<string>();

  // START: App A Update
  if (frame >= 0 && frame < 4) {
      activeModules.add("APP A");
      lines[0] = replaceAt(lines[0], 8, "*WRITE* ");
  }

  // App A -> Local A
  if (frame >= 2 && frame < 6) {
      activeModules.add("APP A");
      const pos = frame - 2; 
      if (pos < 3) lines[1 + pos] = replaceAt(lines[1 + pos], 11, "●");
  }

  // Flash Local DB A
  if (frame >= 5 && frame < 8) {
      lines[5] = replaceAt(lines[5], 6, " PROCESSING ");
      activeModules.add("LOCAL DB A");
      activeModules.add("APP A");
  }

  // Local A -> Remote
  if (frame >= 8 && frame < 12) {
      activeModules.add("LOCAL DB A");
      const pos = frame - 8;
      // Animate down to Remote
      if (pos < 3) lines[7 + pos] = replaceAt(lines[7 + pos], 11, "●");
  }

  // Remote -> Events (Internal)
  if (frame >= 11 && frame < 15) {
     activeModules.add("REMOTE SURREALDB");
     activeModules.add("EVENTS");
  }

  // Events -> Processor (External Down)
  if (frame >= 14 && frame < 20) {
      activeModules.add("REMOTE SURREALDB");
      activeModules.add("EVENTS");
      activeModules.add("SPOOKY STREAM PROCESSOR");
      const pos = frame - 14;
      // Animate vertically down from Events (approx col 12)
      // Path: 16 (inside), 17 (border), 18, 19 -> 20 (Processor Top)
      // pos 0 -> 16
      if (pos < 4) lines[16 + pos] = replaceAt(lines[16 + pos], 12, "●");
  }
  
  // Processor Process
  if (frame >= 18 && frame < 22) {
      activeModules.add("SPOOKY STREAM PROCESSOR");
      // Full width overwrite: 44 chars between the pipes
      // "COMPUTING..." is 12 chars. (44-12)/2 = 16.
      lines[21] = replaceAt(lines[21], 5, "                COMPUTING...                ");
  }

  // Processor -> Incantations (External Up)
  if (frame >= 21 && frame < 27) {
      activeModules.add("SPOOKY STREAM PROCESSOR");
      activeModules.add("INCANTATIONS");
      activeModules.add("REMOTE SURREALDB");
      // Up from 20/19 to 16
      // Path: 19, 18, 17, 16 
      // Arrow is at index 39 (centered under INCANTATIONS)
      const pos = frame - 21;
      if (pos < 4) lines[19 - pos] = replaceAt(lines[19 - pos], 39, "●");
  }

  // Incantations -> Local B (Remote Up)
  if (frame >= 26 && frame < 30) {
      activeModules.add("REMOTE SURREALDB");
      activeModules.add("INCANTATIONS");
      const pos = frame - 26;
      // Up from 10 to 7
      if (pos < 3) lines[9 - pos] = replaceAt(lines[9 - pos], 41, "●");
  }

  // Flash Local DB B
  if (frame >= 29 && frame < 33) {
      lines[5] = replaceAt(lines[5], 36, " UPDATING.. ");
      activeModules.add("LOCAL DB B");
  }

  // Local B -> App B
  if (frame >= 32 && frame < 36) {
      const pos = frame - 32;
      // Up from 4 to 1
      if (pos < 3) lines[3 - pos] = replaceAt(lines[3 - pos], 41, "●");
      activeModules.add("LOCAL DB B");
  }
  
  // Flash App B
  if (frame >= 35 && frame < 39) {
      lines[0] = replaceAt(lines[0], 38, "*UPDATE*");
      activeModules.add("APP B");
  }

  // Helper to colorize the output
  const processLine = (line: string) => {
    // 1. ESCAPE
    const escaped = line
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");

    // 2. COLORIZE
    let processed = escaped
      // Structure (Box borders) - robust regex for any non-space content
      .replace(/(\+[^ ]+\+)/g, '<span class="text-gray-600">$1</span>')
      // Vertical lines
      .replace(/(\|)/g, '<span class="text-gray-600">$1</span>')
      .replace(/(│)/g, '<span class="text-gray-600">$1</span>')
      
      // Arrows
      .replace(/(▼|▲)/g, (match) => {
         return `<span class="text-gray-600">${match}</span>`;
      })
      
      // Statuses
      .replace(/PROCESSING/g, '<span class="text-yellow-400 animate-pulse">PROCESSING</span>')
      .replace(/COMPUTING.../g, '<span class="text-green-400 animate-pulse">COMPUTING...</span>')
      .replace(/UPDATING../g, '<span class="text-green-400 animate-pulse">UPDATING..</span>')
      .replace(/\*UPDATE\*/g, '<span class="text-green-400 font-bold bg-green-900/30">*UPDATE*</span>')
      .replace(/\*WRITE\*/g, '<span class="text-blue-400 font-bold bg-blue-900/30">*WRITE*</span>')

      // The Dot (Data Packet)
      .replace(/●/g, '<span class="text-blue-400 font-bold inline-block text-center">●</span>');

      // CONDITIONAL HIGHLIGHTING
      if (activeModules.has("APP A")) processed = processed.replace(/APP A/g, '<span class="text-blue-400 font-bold">APP A</span>');
      else processed = processed.replace(/APP A/g, '<span class="text-gray-500">APP A</span>');

      if (activeModules.has("APP B")) processed = processed.replace(/APP B/g, '<span class="text-blue-400 font-bold">APP B</span>');
      else processed = processed.replace(/APP B/g, '<span class="text-gray-500">APP B</span>');

      if (activeModules.has("LOCAL DB A")) processed = processed.replace(/LOCAL DB A/g, '<span class="text-purple-400 font-bold">LOCAL DB A</span>');
      else processed = processed.replace(/LOCAL DB A/g, '<span class="text-gray-500">LOCAL DB A</span>');

      if (activeModules.has("LOCAL DB B")) processed = processed.replace(/LOCAL DB B/g, '<span class="text-purple-400 font-bold">LOCAL DB B</span>');
      else processed = processed.replace(/LOCAL DB B/g, '<span class="text-gray-500">LOCAL DB B</span>');

      if (activeModules.has("REMOTE SURREALDB")) processed = processed.replace(/REMOTE SURREALDB/g, '<span class="text-orange-500 font-bold">REMOTE SURREALDB</span>');
      else processed = processed.replace(/REMOTE SURREALDB/g, '<span class="text-gray-500">REMOTE SURREALDB</span>');

      if (activeModules.has("EVENTS")) processed = processed.replace(/EVENTS/g, '<span class="text-yellow-400 font-bold">EVENTS</span>');
      else processed = processed.replace(/EVENTS/g, '<span class="text-gray-500">EVENTS</span>');

      if (activeModules.has("INCANTATIONS")) processed = processed.replace(/INCANTATIONS/g, '<span class="text-pink-500 font-bold">INCANTATIONS</span>');
      else processed = processed.replace(/INCANTATIONS/g, '<span class="text-gray-500">INCANTATIONS</span>');

      if (activeModules.has("SPOOKY STREAM PROCESSOR")) processed = processed.replace(/SPOOKY STREAM PROCESSOR/g, '<span class="text-green-500 font-bold">SPOOKY STREAM PROCESSOR</span>');
      else processed = processed.replace(/SPOOKY STREAM PROCESSOR/g, '<span class="text-gray-500">SPOOKY STREAM PROCESSOR</span>');

      return processed;
  };

  return (
    <div className="flex justify-center w-full">
         <div className="font-mono text-[10px] xs:text-xs leading-tight font-bold whitespace-pre overflow-x-auto select-none bg-black p-6 rounded border border-[#333] shadow-2xl inline-block">
            <div className="text-gray-500 mb-4 border-b border-[#333] pb-2 text-center tracking-widest uppercase">
                // SYSTEM_ARCHITECTURE_LIVE_VIEW
            </div>
            {lines.map((l, i) => (
                <div key={i} dangerouslySetInnerHTML={{ __html: processLine(l) }} />
            ))}
        </div>
    </div>
  );
};

