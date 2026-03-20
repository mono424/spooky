import React, { useEffect, useRef, useState } from 'react';

// ---------- Back Slice: ASCII Architecture Animation ----------
const ArchitectureBackSlice = ({ animFrame }: { animFrame: number }) => {
  const replaceAt = (str: string, index: number, replacement: string) => {
    return str.substring(0, index) + replacement + str.substring(index + replacement.length);
  };

  const baseLines = [
    '      [   APP A  ]                  [   APP B  ]      ', // 0
    '           ▲                             ▲            ', // 1
    '           │                             │            ', // 2
    '           ▼                             ▼            ', // 3
    '    +--------------+              +--------------+    ', // 4
    '    |  LOCAL DB A  |              |  LOCAL DB B  |    ', // 5
    '    +--------------+              +--------------+    ', // 6
    '           ▲                             ▲            ', // 7
    '           │                             │            ', // 8
    '           ▼                             ▼            ', // 9
    '    +--------------------------------------------+    ', // 10
    '    |             REMOTE SURREALDB               |    ', // 11
    '    |                                            |    ', // 12
    '    |  +----------+            +--------------+  |    ', // 13
    '    |  |  EVENTS  |            |   QUERIES    |  |    ', // 14
    '    |  +----------+            +--------------+  |    ', // 15
    '    |       │                          ▲         |    ', // 16
    '    +-------│--------------------------│---------+    ', // 17
    '            │                          │              ', // 18
    '            ▼                          │              ', // 19
    '    +--------------------------------------------+    ', // 20
    '    |           SPOOKY STREAM PROCESSOR          |    ', // 21
    '    +--------------------------------------------+    ', // 22
  ];

  const frame = animFrame;
  const lines = [...baseLines];
  const activeModules = new Set<string>();

  if (frame >= 0 && frame < 4) {
    activeModules.add('APP A');
    lines[0] = replaceAt(lines[0], 8, '*WRITE* ');
  }

  if (frame >= 2 && frame < 6) {
    activeModules.add('APP A');
    const pos = frame - 2;
    if (pos < 3) lines[1 + pos] = replaceAt(lines[1 + pos], 11, '●');
  }

  if (frame >= 5 && frame < 8) {
    lines[5] = replaceAt(lines[5], 6, ' PROCESSING ');
    activeModules.add('LOCAL DB A');
    activeModules.add('APP A');
  }

  if (frame >= 8 && frame < 12) {
    activeModules.add('LOCAL DB A');
    const pos = frame - 8;
    if (pos < 3) lines[7 + pos] = replaceAt(lines[7 + pos], 11, '●');
  }

  if (frame >= 11 && frame < 15) {
    activeModules.add('REMOTE SURREALDB');
    activeModules.add('EVENTS');
  }

  if (frame >= 14 && frame < 20) {
    activeModules.add('REMOTE SURREALDB');
    activeModules.add('EVENTS');
    activeModules.add('SPOOKY STREAM PROCESSOR');
    const pos = frame - 14;
    if (pos < 4) lines[16 + pos] = replaceAt(lines[16 + pos], 12, '●');
  }

  if (frame >= 18 && frame < 22) {
    activeModules.add('SPOOKY STREAM PROCESSOR');
    lines[21] = replaceAt(lines[21], 5, '                COMPUTING...                ');
  }

  if (frame >= 21 && frame < 27) {
    activeModules.add('SPOOKY STREAM PROCESSOR');
    activeModules.add('QUERIES');
    activeModules.add('REMOTE SURREALDB');
    const pos = frame - 21;
    if (pos < 4) lines[19 - pos] = replaceAt(lines[19 - pos], 39, '●');
  }

  if (frame >= 26 && frame < 30) {
    activeModules.add('REMOTE SURREALDB');
    activeModules.add('QUERIES');
    const pos = frame - 26;
    if (pos < 3) lines[9 - pos] = replaceAt(lines[9 - pos], 41, '●');
  }

  if (frame >= 29 && frame < 33) {
    lines[5] = replaceAt(lines[5], 36, ' UPDATING.. ');
    activeModules.add('LOCAL DB B');
  }

  if (frame >= 32 && frame < 36) {
    const pos = frame - 32;
    if (pos < 3) lines[3 - pos] = replaceAt(lines[3 - pos], 41, '●');
    activeModules.add('LOCAL DB B');
  }

  if (frame >= 35 && frame < 39) {
    lines[0] = replaceAt(lines[0], 38, '*UPDATE*');
    activeModules.add('APP B');
  }

  const processLine = (line: string) => {
    const escaped = line.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

    let processed = escaped
      .replace(/(\+[^ ]+\+)/g, '<span class="text-gray-600">$1</span>')
      .replace(/(\|)/g, '<span class="text-gray-600">$1</span>')
      .replace(/(│)/g, '<span class="text-gray-600">$1</span>')
      .replace(/(▼|▲)/g, '<span class="text-gray-600">$&</span>')
      .replace(/PROCESSING/g, '<span class="text-yellow-400 animate-pulse">PROCESSING</span>')
      .replace(/COMPUTING.../g, '<span class="text-green-400 animate-pulse">COMPUTING...</span>')
      .replace(/UPDATING../g, '<span class="text-green-400 animate-pulse">UPDATING..</span>')
      .replace(
        /\*UPDATE\*/g,
        '<span class="text-green-400 font-bold bg-green-900/30">*UPDATE*</span>'
      )
      .replace(/\*WRITE\*/g, '<span class="text-blue-400 font-bold bg-blue-900/30">*WRITE*</span>')
      .replace(/●/g, '<span class="text-blue-400 font-bold inline-block text-center">●</span>');

    if (activeModules.has('APP A'))
      processed = processed.replace(/APP A/g, '<span class="text-blue-400 font-bold">APP A</span>');
    else processed = processed.replace(/APP A/g, '<span class="text-gray-500">APP A</span>');

    if (activeModules.has('APP B'))
      processed = processed.replace(/APP B/g, '<span class="text-blue-400 font-bold">APP B</span>');
    else processed = processed.replace(/APP B/g, '<span class="text-gray-500">APP B</span>');

    if (activeModules.has('LOCAL DB A'))
      processed = processed.replace(
        /LOCAL DB A/g,
        '<span class="text-purple-400 font-bold">LOCAL DB A</span>'
      );
    else
      processed = processed.replace(/LOCAL DB A/g, '<span class="text-gray-500">LOCAL DB A</span>');

    if (activeModules.has('LOCAL DB B'))
      processed = processed.replace(
        /LOCAL DB B/g,
        '<span class="text-purple-400 font-bold">LOCAL DB B</span>'
      );
    else
      processed = processed.replace(/LOCAL DB B/g, '<span class="text-gray-500">LOCAL DB B</span>');

    if (activeModules.has('REMOTE SURREALDB'))
      processed = processed.replace(
        /REMOTE SURREALDB/g,
        '<span class="text-orange-500 font-bold">REMOTE SURREALDB</span>'
      );
    else
      processed = processed.replace(
        /REMOTE SURREALDB/g,
        '<span class="text-gray-500">REMOTE SURREALDB</span>'
      );

    if (activeModules.has('EVENTS'))
      processed = processed.replace(
        /EVENTS/g,
        '<span class="text-yellow-400 font-bold">EVENTS</span>'
      );
    else processed = processed.replace(/EVENTS/g, '<span class="text-gray-500">EVENTS</span>');

    if (activeModules.has('QUERIES'))
      processed = processed.replace(
        /QUERIES/g,
        '<span class="text-pink-500 font-bold">QUERIES</span>'
      );
    else
      processed = processed.replace(
        /QUERIES/g,
        '<span class="text-gray-500">QUERIES</span>'
      );

    if (activeModules.has('SPOOKY STREAM PROCESSOR'))
      processed = processed.replace(
        /SPOOKY STREAM PROCESSOR/g,
        '<span class="text-green-500 font-bold">SPOOKY STREAM PROCESSOR</span>'
      );
    else
      processed = processed.replace(
        /SPOOKY STREAM PROCESSOR/g,
        '<span class="text-gray-500">SPOOKY STREAM PROCESSOR</span>'
      );

    return processed;
  };

  return (
    <div className="font-mono text-[10px] sm:text-xs leading-tight font-bold whitespace-pre overflow-x-auto select-none bg-black p-6 rounded border border-[#333] shadow-2xl inline-block">
      <div className="text-gray-500 mb-4 border-b border-[#333] pb-2 text-center tracking-widest uppercase">
        // SYSTEM_ARCHITECTURE_LIVE_VIEW
      </div>
      {lines.map((l, i) => (
        <div key={i} dangerouslySetInnerHTML={{ __html: processLine(l) }} />
      ))}
    </div>
  );
};

// ---------- Funny task pool ----------
const TASK_POOL = [
  'Deploy to production on Friday',
  'Fix bug that fixes itself',
  'Mass delete everything',
  'Refactor the refactoring',
  'Add AI to the AI',
  'Update the update script',
  'Delete node_modules again',
  'Rewrite it in Rust',
  'Blame the intern',
  'Google the error message',
  'Add blockchain somewhere',
  'Undo last undo',
  'Make it pop more',
  'Turn it off and on again',
  'Pretend to understand regex',
  'Ship it and pray',
  'Rename everything to final_v2',
  'Close 47 browser tabs',
  'Ask ChatGPT to explain',
  'Push directly to main',
  'Ignore the tech debt',
  'Add more console.logs',
  'Read the documentation lol',
  'Meetings about meetings',
  'Cache invalidation',
  'Center the div',
  'Fix CSS on Safari',
  'Migrate to the new new thing',
  'Write tests (maybe later)',
  'Convince PM scope is too big',
];

type TaskItem = { id: number; text: string };

const ROW_HEIGHT = 40; // px per task row
const VISIBLE_COUNT = 3;
const TIME_LABELS = ['2m ago', '5m ago', '12m ago'];

const TaskRow = ({ task, isNew, time }: { task: TaskItem; isNew?: boolean; time: string }) => (
  <div
    className="flex items-center justify-between px-4 border-b border-[#222]"
    style={{
      height: `${ROW_HEIGHT}px`,
      background: isNew ? 'rgba(5, 46, 22, 0.4)' : 'transparent',
    }}
  >
    <div className="flex items-center gap-2.5 min-w-0">
      {isNew ? (
        <span className="w-4 h-4 rounded border border-[#555] flex-shrink-0" />
      ) : (
        <span className="w-4 h-4 rounded border border-[#555] flex-shrink-0 flex items-center justify-center bg-[#111]">
          <span className="text-green-400 text-[10px] leading-none">&#10003;</span>
        </span>
      )}
      <span className={isNew ? 'text-green-400 font-medium truncate' : 'text-gray-400 line-through truncate'}>
        {task.text}
      </span>
    </div>
    <span className={`text-xs flex-shrink-0 ml-2 ${isNew ? 'text-green-500/70' : 'text-gray-600'}`}>
      {time}
    </span>
  </div>
);

// ---------- Front Slice: Task List Mockup ----------
const FrontAppMockup = ({ animFrame, cycle }: { animFrame: number; cycle: number }) => {
  // The stack is the source of truth. Newest task at index 0.
  const [stack, setStack] = useState<TaskItem[]>([
    { id: 2, text: TASK_POOL[2] },
    { id: 1, text: TASK_POOL[1] },
    { id: 0, text: TASK_POOL[0] },
  ]);
  // Whether the slide transition is enabled (disabled during snap-back)
  const [animate, setAnimate] = useState(true);
  const committedCycleRef = useRef(0);

  const isSliding = animFrame >= 35;

  // When a new cycle starts (animFrame wraps to 0), commit the task that
  // just slid in. We snap the wrapper back to its resting position instantly
  // (no transition) so the newly committed row stays in place visually.
  useEffect(() => {
    if (cycle > committedCycleRef.current) {
      committedCycleRef.current = cycle;
      const newId = 2 + cycle;
      const poolIdx = newId % TASK_POOL.length;
      // 1. Disable transition so the snap-back is instant
      setAnimate(false);
      // 2. Push the task into the stack
      setStack((prev) => [{ id: newId, text: TASK_POOL[poolIdx] }, ...prev]);
    }
  }, [cycle]);

  // Re-enable transitions one frame after the snap-back
  useEffect(() => {
    if (!animate) {
      const raf = requestAnimationFrame(() => {
        setAnimate(true);
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [animate]);

  // The next task that will slide in (not yet in stack)
  const pendingId = 2 + cycle + 1;
  const pendingPoolIdx = pendingId % TASK_POOL.length;
  const pendingTask: TaskItem = { id: pendingId, text: TASK_POOL[pendingPoolIdx] };

  // We render: [pending, stack[0], stack[1], stack[2], stack[3]]
  // Container clips to VISIBLE_COUNT rows.
  // Resting state: wrapper is at translateY(-ROW_HEIGHT) hiding the pending row above.
  // Sliding state: wrapper moves to translateY(0), pushing the pending row into view
  // and the bottom row out below the clip.
  const renderItems = [pendingTask, ...stack.slice(0, VISIBLE_COUNT)];

  return (
    <div className="bg-black border border-[#333] rounded shadow-2xl text-sm w-full max-w-[380px] font-sans select-none">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-[#333]">
        <span className="text-white font-semibold text-base">Tasks</span>
        <button className="text-xs text-gray-400 border border-[#444] rounded px-2 py-0.5 hover:border-gray-500 transition-colors">
          + Add Task
        </button>
      </div>

      {/* Task list — fixed height clip */}
      <div className="overflow-hidden" style={{ height: `${VISIBLE_COUNT * ROW_HEIGHT}px` }}>
        <div
          style={{
            transform: `translateY(${isSliding ? 0 : -ROW_HEIGHT}px)`,
            transition: animate ? 'transform 600ms cubic-bezier(0.4, 0, 0.2, 1)' : 'none',
          }}
        >
          {renderItems.map((task, i) => (
            <TaskRow
              key={task.id}
              task={task}
              isNew={i === 0 && isSliding}
              time={i === 0 ? 'just now' : TIME_LABELS[i - 1] ?? '20m+'}
            />
          ))}
        </div>
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between px-4 py-2 border-t border-[#333] text-xs text-gray-500">
        <span>
          <span
            className={`inline-block w-1.5 h-1.5 rounded-full mr-1.5 ${isSliding ? 'bg-green-400 animate-pulse' : 'bg-green-600'}`}
          />
          synced &middot; {stack.length + (isSliding ? 1 : 0)} tasks
        </span>
        <span className="text-green-600">connected</span>
      </div>
    </div>
  );
};

// ---------- Parent: 3D Scene ----------
export const ArchitectureDiagram = () => {
  const [frame, setFrame] = useState(0);
  const [hovered, setHovered] = useState(false);

  useEffect(() => {
    const timer = setInterval(() => {
      setFrame((f) => f + 1);
    }, 150);
    return () => clearInterval(timer);
  }, []);

  const animFrame = frame % 40;
  const cycle = Math.floor(frame / 40);

  return (
    <div className="flex justify-center w-full">
      {/* Desktop: 3D layered view */}
      <div
        className="relative w-full max-w-4xl hidden lg:block"
        style={{ perspective: '1200px' }}
      >
        <div
          className="relative"
          style={{ transformStyle: 'preserve-3d' }}
        >
          {/* Back Slice: Architecture diagram */}
          <div
            className="absolute top-0 right-0 flex justify-center w-full transition-all duration-500 ease-out"
            style={{
              transform: hovered
                ? 'rotateY(-2deg) rotateX(1deg) translateZ(20px)'
                : 'rotateY(-8deg) rotateX(3deg) translateZ(-60px)',
              opacity: hovered ? 1 : 0.85,
              top: hovered ? '0px' : '-20px',
              right: hovered ? '0px' : '-10px',
              zIndex: hovered ? 20 : 0,
            }}
            onMouseEnter={() => setHovered(true)}
            onMouseLeave={() => setHovered(false)}
          >
            <ArchitectureBackSlice animFrame={animFrame} />
          </div>

          {/* Front Slice: App mockup */}
          <div
            className="relative z-10 pl-8 transition-all duration-500 ease-out"
            style={{
              transform: hovered
                ? 'rotateY(-8deg) rotateX(3deg) translateZ(-40px) scale(0.95)'
                : 'rotateY(-8deg) rotateX(3deg) translateZ(40px)',
              opacity: hovered ? 0.6 : 1,
              width: '85%',
              zIndex: hovered ? 0 : 10,
            }}
          >
            <FrontAppMockup animFrame={animFrame} cycle={cycle} />
          </div>
        </div>
      </div>

      {/* Mobile: stacked layout without 3D */}
      <div className="lg:hidden flex flex-col items-center gap-6 w-full">
        <FrontAppMockup animFrame={animFrame} cycle={cycle} />
        <ArchitectureBackSlice animFrame={animFrame} />
      </div>
    </div>
  );
};
