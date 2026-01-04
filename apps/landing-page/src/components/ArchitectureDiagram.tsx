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

  // Animation State
  const activeModules = new Set<string>();

  // START: Client A Update
  if (frame >= 0 && frame < 4) {
      activeModules.add("CLIENT A");
      lines[0] = replaceAt(lines[0], 8, "*WRITE* "); // Replaces "CLIENT A" (8 chars) with "*WRITE* " (8 chars)
  }

  // A -> Local A
  if (frame >= 2 && frame < 6) {
      activeModules.add("CLIENT A");
      const pos = frame - 2; 
      if (pos < 3) lines[1 + pos] = replaceAt(lines[1 + pos], 11, "§");
  }

  // Flash Local DB A
  if (frame >= 5 && frame < 8) {
      lines[5] = replaceAt(lines[5], 6, " PROCESSING ");
      activeModules.add("LOCAL DB A");
      // keep client active?
      activeModules.add("CLIENT A");
  }

  // Local A -> Remote
  if (frame >= 8 && frame < 11) {
      const pos = frame - 8;
      if (pos < 3) lines[7 + pos] = replaceAt(lines[7 + pos], 11, "§");
      activeModules.add("LOCAL DB A");
  }

  // Flash Remote (Ingest) -> Internal Move
  if (frame >= 11 && frame < 20) {
     lines[11] = replaceAt(lines[11], 18, "~~ SPOOKY STUFF ~~");
     activeModules.add("REMOTE SURREALDB");
     activeModules.add("INCANTATIONS");
     activeModules.add("DBSP MODULE");
     
     const arrowStart = 29;
     const arrowEnd = 23;
     const steps = 6;
     const progress = Math.max(0, Math.min(steps, frame - 13));
     
     if (progress < steps) {
        lines[14] = replaceAt(lines[14], arrowStart - progress, "§");
     }
  }

  // Remote -> Local B
  if (frame >= 20 && frame < 24) {
      const pos = frame - 20;
      if (pos < 3) lines[9 - pos] = replaceAt(lines[9 - pos], 41, "§");
      activeModules.add("REMOTE SURREALDB");
  }

  // Flash Local DB B
  if (frame >= 23 && frame < 26) {
      lines[5] = replaceAt(lines[5], 36, " UPDATING.. ");
      activeModules.add("LOCAL DB B");
  }

  // Local B -> Client B
  if (frame >= 26 && frame < 30) {
      const pos = frame - 26;
      if (pos < 3) lines[3 - pos] = replaceAt(lines[3 - pos], 41, "§");
      activeModules.add("LOCAL DB B");
  }
  
  // Flash Client B
  if (frame >= 29 && frame < 34) {
      lines[0] = replaceAt(lines[0], 38, "*UPDATE*");
      activeModules.add("CLIENT B");
  }

  // Helper to colorize the output
  const processLine = (line: string) => {
    // 1. ESCAPE: First escape HTML entities to prevent broken rendering
    const escaped = line
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");

    // 2. COLORIZE: Apply colors to the ESCAPED string
    let processed = escaped
      // Structure (Box borders)
      .replace(/(\+[-]+\+)/g, '<span class="text-gray-600">$1</span>')
      .replace(/(\|)/g, '<span class="text-gray-600">$1</span>')
      
      // Arrows
      .replace(/(v|\|&lt;-{2,}\|arg|\|&lt;-{2,}\|)/g, (match) => {
         if (match === 'v') return '<span class="text-gray-600">v</span>';
         return `<span class="text-gray-600">${match}</span>`;
      })
      .replace(/\|&lt;-+-+\|/g, '<span class="text-gray-600">$&</span>') 
      
      // Statuses
      .replace(/PROCESSING/g, '<span class="text-yellow-400 animate-pulse">PROCESSING</span>')
      .replace(/UPDATING../g, '<span class="text-green-400 animate-pulse">UPDATING..</span>')
      .replace(/SPOOKY STUFF/g, '<span class="text-purple-500 font-bold">SPOOKY STUFF</span>')
      .replace(/\*UPDATE\*/g, '<span class="text-green-400 font-bold bg-green-900/30">*UPDATE*</span>')
      .replace(/\*WRITE\*/g, '<span class="text-blue-400 font-bold bg-blue-900/30">*WRITE*</span>')

      // The Dot (Data Packet)
      .replace(/§/g, '<span class="text-blue-400 font-bold inline-block w-[1ch] text-center">●</span>');

      // CONDITIONAL HIGHLIGHTING
      if (activeModules.has("CLIENT A")) processed = processed.replace(/CLIENT A/g, '<span class="text-blue-400 font-bold">CLIENT A</span>');
      else processed = processed.replace(/CLIENT A/g, '<span class="text-gray-500">CLIENT A</span>');

      if (activeModules.has("CLIENT B")) processed = processed.replace(/CLIENT B/g, '<span class="text-blue-400 font-bold">CLIENT B</span>');
      else processed = processed.replace(/CLIENT B/g, '<span class="text-gray-500">CLIENT B</span>');

      if (activeModules.has("LOCAL DB A")) processed = processed.replace(/LOCAL DB A/g, '<span class="text-purple-400 font-bold">LOCAL DB A</span>');
      else processed = processed.replace(/LOCAL DB A/g, '<span class="text-gray-500">LOCAL DB A</span>');

      if (activeModules.has("LOCAL DB B")) processed = processed.replace(/LOCAL DB B/g, '<span class="text-purple-400 font-bold">LOCAL DB B</span>');
      else processed = processed.replace(/LOCAL DB B/g, '<span class="text-gray-500">LOCAL DB B</span>');

      if (activeModules.has("REMOTE SURREALDB")) processed = processed.replace(/REMOTE SURREALDB/g, '<span class="text-orange-500 font-bold">REMOTE SURREALDB</span>');
      else processed = processed.replace(/REMOTE SURREALDB/g, '<span class="text-gray-500">REMOTE SURREALDB</span>');

      if (activeModules.has("INCANTATIONS")) processed = processed.replace(/INCANTATIONS/g, '<span class="text-pink-500 font-bold">INCANTATIONS</span>');
      else processed = processed.replace(/INCANTATIONS/g, '<span class="text-gray-500">INCANTATIONS</span>');

      if (activeModules.has("DBSP MODULE")) processed = processed.replace(/DBSP MODULE/g, '<span class="text-green-500 font-bold">DBSP MODULE</span>');
      else processed = processed.replace(/DBSP MODULE/g, '<span class="text-gray-500">DBSP MODULE</span>');

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

