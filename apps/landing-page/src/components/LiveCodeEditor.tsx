import React, { useState, useRef } from "react";

interface LiveCodeEditorProps {
  initialCode: string;
  filename?: string;
}

// --- 1. DATA & TYPES ---

type SuggestionItem = {
  label: string;
  kind: "field" | "table" | "keyword" | "relation";
  type?: string;
  doc?: string;
};

const GROUPS = {
  TABLES: [
    { label: "user", kind: "table", type: "SCHEMAFULL", doc: "System users and authentication details." },
    { label: "thread", kind: "table", type: "SCHEMAFULL", doc: "Root collection for discussion threads. Contains title, body, and author references." },
    { label: "notification", kind: "table", type: "SCHEMAFULL", doc: "User activity alerts." },
  ] as SuggestionItem[],
  RELATIONS: [
    { label: "author", kind: "relation", type: "record<user>", doc: "The creator of the thread." },
    { label: "comments", kind: "relation", type: "array<comment>", doc: "Replies to this thread." },
    { label: "categories", kind: "relation", type: "array<category>", doc: "Tags associated with thread." },
  ] as SuggestionItem[],
  ORDER_FIELDS: [
    { label: "created_at", kind: "field", type: "datetime", doc: "Creation timestamp." },
    { label: "updated_at", kind: "field", type: "datetime", doc: "Last modification." },
    { label: "title", kind: "field", type: "string", doc: "Alphabetical sort." },
    { label: "score", kind: "field", type: "int", doc: "Popularity metric." },
  ] as SuggestionItem[],
  DIRECTIONS: [
    { label: "asc", kind: "keyword", doc: "Ascending (A-Z, 0-9)." },
    { label: "desc", kind: "keyword", doc: "Descending (Z-A, 9-0)." },
  ] as SuggestionItem[],
};

const getContextForValue = (val: string) => {
  if (GROUPS.TABLES.find(i => i.label === val)) return { options: GROUPS.TABLES, title: "Table Schema", isReadOnly: true };
  if (GROUPS.RELATIONS.find(i => i.label === val)) return { options: GROUPS.RELATIONS, title: "Select Relation", isReadOnly: false };
  if (GROUPS.ORDER_FIELDS.find(i => i.label === val)) return { options: GROUPS.ORDER_FIELDS, title: "Order By", isReadOnly: false };
  if (GROUPS.DIRECTIONS.find(i => i.label === val)) return { options: GROUPS.DIRECTIONS, title: "Direction", isReadOnly: false };
  return null;
};

const Icons = {
  field: (
    <svg className="w-3 h-3 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
    </svg>
  ),
  table: (
    <svg className="w-3 h-3 text-orange-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 10h18M3 14h18m-9-4v8m-7-4h14M2 5h20v14H2V5z" />
    </svg>
  ),
  relation: (
    <svg className="w-3 h-3 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
    </svg>
  ),
  keyword: (
    <svg className="w-3 h-3 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 7h.01M7 3h5c.512 0 1.024.195 1.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.994 1.994 0 013 12V7a4 4 0 014-4z" />
    </svg>
  ),
  error: (
    <svg className="w-3 h-3 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
    </svg>
  )
};

// --- 2. SUB-COMPONENTS ---

interface TokenProps {
  text: string;
  className: string;
  cleanText: string;
  onReplace: (oldVal: string, newVal: string) => void;
}

const Token: React.FC<TokenProps> = ({ text, className, cleanText, onReplace }) => {
  const [isOpen, setIsOpen] = useState(false);
  const timeoutRef = useRef<number | null>(null);
  const context = getContextForValue(cleanText);

  if (!context) {
    return <span className={className} dangerouslySetInnerHTML={{__html: text}} />;
  }

  const currentItem = context.options.find(o => o.label === cleanText);

  // --- HOVER LOGIC ---
  const handleEnter = () => {
    if (timeoutRef.current) window.clearTimeout(timeoutRef.current);
    setIsOpen(true);
  };

  const handleLeave = () => {
    timeoutRef.current = window.setTimeout(() => {
      setIsOpen(false);
    }, 150);
  };

  const zIndexClass = isOpen ? "z-[9999] relative" : "z-auto relative";

  // --- STYLE 1: READ-ONLY INFO ---
  if (context.isReadOnly) {
    return (
      <span 
        className={`${className} inline-block ${zIndexClass}`}
        onMouseEnter={handleEnter}
        onMouseLeave={handleLeave}
      >
        {/* Invisible Bridge */}
        <span className="absolute inset-0 -bottom-2 bg-transparent z-10" />

        <span 
          className="cursor-help decoration-dotted underline decoration-white/30 underline-offset-4 hover:decoration-white/60 hover:text-white transition-all relative z-20"
          dangerouslySetInnerHTML={{__html: text}} 
        />
        
        {isOpen && (
          <div className="absolute left-0 top-full mt-1 w-[280px] bg-[#1a1a1a] border border-[#333] shadow-[0_8px_32px_rgba(0,0,0,0.9)] rounded-sm font-mono text-xs animate-in fade-in slide-in-from-top-1 duration-150 pointer-events-auto cursor-default z-30">
             <div className="bg-[#111] px-3 py-2 border-b border-[#222] flex items-center gap-2">
                {Icons[currentItem?.kind || 'table']}
                <span className="font-bold text-gray-200">{cleanText}</span>
                <span className="text-[10px] text-orange-400 bg-orange-400/10 px-1.5 rounded border border-orange-400/20 ml-auto">
                  {currentItem?.type}
                </span>
             </div>
             <div className="p-3 text-gray-400 leading-relaxed whitespace-normal break-words">
               {currentItem?.doc}
             </div>
             <div className="bg-[#111] px-3 py-1.5 border-t border-[#222] text-[10px] text-gray-600">
                <span className="text-blue-500">★</span> Primary Key: <span className="text-gray-400">id</span>
             </div>
          </div>
        )}
      </span>
    );
  }

  // --- STYLE 2: EDITABLE SELECTOR ---
  return (
    <span 
      className={`${className} inline-block ${zIndexClass}`}
      onMouseEnter={handleEnter}
      onMouseLeave={handleLeave}
    >
      {/* Invisible Bridge: Covers the text and extends down to popover */}
      <span className="absolute inset-0 -bottom-2 bg-transparent z-10" />

      {/* Visual Token: Standard padding ensures clean underline */}
      <span 
        className="cursor-pointer border-b border-dashed border-green-500/40 bg-green-500/5 hover:bg-green-500/10 hover:border-green-400 transition-colors rounded-sm px-0.5 relative z-20"
        dangerouslySetInnerHTML={{__html: text}} 
      />
      
      {isOpen && (
        <div className="absolute left-0 top-full mt-1 min-w-[240px] bg-[#1a1a1a] border border-[#333] shadow-[0_8px_32px_rgba(0,0,0,0.8)] rounded-sm font-mono text-xs animate-in fade-in zoom-in-95 duration-100 overflow-hidden pointer-events-auto z-30">
          <div className="bg-[#111] px-3 py-1.5 text-[10px] text-gray-500 border-b border-[#222] flex justify-between items-center uppercase tracking-wider shrink-0">
             <span className="font-bold text-gray-400">{context.title}</span>
             <span className="bg-[#222] px-1 rounded text-[9px]">Tab ↹</span>
          </div>

          <div className="py-1">
            {context.options.map((opt) => {
              const isActive = opt.label === cleanText;
              return (
                <button
                  key={opt.label}
                  onClick={(e) => {
                    e.stopPropagation();
                    onReplace(cleanText, opt.label);
                    setIsOpen(false);
                  }}
                  className={`w-full text-left px-3 py-2 flex justify-between items-center group transition-colors ${
                    isActive 
                      ? 'bg-[#0f2e1a] text-white border-l-2 border-green-500' 
                      : 'text-gray-400 border-l-2 border-transparent hover:bg-[#222] hover:text-gray-200'
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <span className="opacity-80 shrink-0">{Icons[opt.kind]}</span>
                    <span className={isActive ? 'font-bold' : ''}>{opt.label}</span>
                  </div>
                  {opt.type && (
                    <span className={`text-[10px] ml-2 ${isActive ? 'text-green-400' : 'text-gray-600'}`}>
                      {opt.type}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          {context.options.find(o => o.label === cleanText)?.doc && (
             <div className="bg-[#111] border-t border-[#222] p-2 text-gray-500 text-[10px] leading-relaxed whitespace-normal break-words">
               <span className="text-green-600 font-bold">INFO: </span>
               {context.options.find(o => o.label === cleanText)?.doc}
             </div>
          )}
        </div>
      )}
    </span>
  );
};

// --- 3. ERROR TOKEN COMPONENT ---

const ErrorToken: React.FC<{ text: string }> = ({ text }) => {
    const [isOpen, setIsOpen] = useState(false);
    
    return (
        <span 
            className={`group relative cursor-pointer inline-block ${isOpen ? "z-[9999]" : "z-auto"}`}
            onMouseEnter={() => setIsOpen(true)}
            onMouseLeave={() => setIsOpen(false)}
        >
            <span className="text-gray-300 decoration-wavy underline decoration-red-500 underline-offset-2 relative z-20">
                {text}
            </span>
            {isOpen && (
                <div className="absolute left-0 bottom-full mb-1 w-[300px] bg-[#1a1a1a] border border-red-500/50 shadow-2xl rounded-sm font-mono text-xs z-[100] animate-in fade-in slide-in-from-bottom-1 pointer-events-auto">
                    <div className="bg-red-500/10 px-3 py-2 border-b border-red-500/20 flex items-center gap-2 text-red-400">
                        {Icons.error}
                        <span className="font-bold">Property 'author' does not exist</span>
                    </div>
                    <div className="p-3 text-gray-400 leading-relaxed whitespace-normal break-words">
                        Type <span className="text-orange-300">Thread</span> has no property <span className="text-white">author</span>. 
                        Did you forget to include <span className="bg-[#222] px-1 text-blue-300">.related("author")</span> in your query?
                    </div>
                </div>
            )}
        </span>
    )
}


// --- 4. MAIN COMPONENT ---

export const LiveCodeEditor: React.FC<LiveCodeEditorProps> = ({ initialCode, filename }) => {
  const [code, setCode] = useState(initialCode);
  const [currentRelation, setCurrentRelation] = useState("comments");

  const handleReplace = (oldVal: string, newVal: string) => {
    setCode(prev => prev.replace(`"${oldVal}"`, `"${newVal}"`));
    
    if (GROUPS.RELATIONS.find(r => r.label === oldVal)) {
        setCurrentRelation(newVal);
    }
  };

  const renderCode = (text: string) => {
    const tokens = text.split(/(".*?"|\/\/.*$)/gm);

    return tokens.map((token, index) => {
      // A. Strings (Intellisense)
      if (token.startsWith('"')) {
        const content = token.slice(1, -1);
        const safeText = token.replace(/</g, "&lt;").replace(/>/g, "&gt;");
        return (
          <Token 
            key={index} 
            text={safeText} 
            className="text-orange-300" 
            cleanText={content}
            onReplace={handleReplace}
          />
        );
      }
      
      // B. Comments
      if (token.startsWith('//')) {
        return <span key={index} className="text-gray-500 italic">{token}</span>;
      }
      
      // C. Code Parts
      const parts = token.split(/(thread\.author\.username|\b(?:useQuery|db|query|related|orderBy|limit|build|import|export|from|const|return|function|default|type)\b|[{}[\]().,;]|\s+)/g);
      
      return (
          <span key={index}>
              {parts.map((part, i) => {
                  if (!part) return null;

                  if (part === "thread.author.username") {
                      if (currentRelation !== "author") {
                        return <ErrorToken key={i} text="thread.author.username" />;
                      } else {
                        return (
                            <span key={i}>
                                <span className="text-gray-300">thread</span>
                                <span className="text-gray-500">.</span>
                                <span className="text-gray-300">author</span>
                                <span className="text-gray-500">.</span>
                                <span className="text-gray-300">username</span>
                            </span>
                        );
                      }
                  }

                  if (part.match(/\b(import|export|from|const|return|function|default|type)\b/)) {
                      return <span key={i} className="text-purple-400 font-bold">{part}</span>;
                  }
                  if (part.match(/\b(useQuery|db|query|related|orderBy|limit|build)\b/)) {
                      return <span key={i} className="text-blue-400">{part}</span>;
                  }
                  if (part.match(/<[A-Z][a-zA-Z]*|<div|<span|<p\b/) || part.match(/<\/[A-Za-z]+>/)) {
                     return <span key={i} className="text-yellow-300">{part}</span>;
                  }
                  if (part.match(/[{}[\]().,;]/) || part.match(/[<>]/)) { 
                     return <span key={i} className="text-gray-500">{part}</span>;
                  }
                  
                  return part;
              })}
          </span>
      );
    });
  };

  const lineCount = code.split("\n").length;

  return (
    <div className="border border-[#333] bg-[#050505] shadow-2xl flex flex-col font-mono text-sm relative overflow-visible group rounded-sm min-h-[420px]">
      
      {/* Header */}
      <div className="flex justify-between items-center bg-[#111] border-b border-[#222] px-4 pt-2 text-xs select-none">
        <div className="flex items-center gap-2">
            <div className="flex gap-1.5 pb-1">
                <div className="w-2.5 h-2.5 rounded-full bg-[#333]"></div>
                <div className="w-2.5 h-2.5 rounded-full bg-[#333]"></div>
                <div className="w-2.5 h-2.5 rounded-full bg-[#333]"></div>
            </div>
            <div className="flex items-center gap-2 text-gray-300 bg-[#050505] px-3 py-1 border-t-2 border-green-700 ml-4">
               <span className="font-bold text-blue-400">TSX</span> 
               <span>{filename || 'Untitled'}</span>
            </div>
        </div>
        
        <div className={`text-[10px] font-bold px-2 py-0.5 rounded transition-colors ${currentRelation !== 'author' ? 'bg-red-900/20 text-red-500' : 'text-green-900/0'}`}>
            {currentRelation !== 'author' ? '1 ERROR' : ''}
        </div>
      </div>

      {/* Body */}
      <div className="relative flex-1 flex min-h-0">
            {/* Gutter */}
            <div className="bg-[#0a0a0a] border-r border-[#222] text-[#444] text-right py-4 pr-3 pl-2 select-none w-12 shrink-0 font-mono leading-relaxed">
            {Array.from({ length: lineCount }).map((_, i) => (
                <div key={i}>{i + 1}</div>
            ))}
            </div>

            {/* Code */}
            <div className="relative flex-1 py-4 px-4 overflow-visible cursor-text">
                <pre className="m-0 p-0 whitespace-pre font-mono leading-relaxed text-gray-300">
                    {renderCode(code)}
                </pre>
            </div>
      </div>
      
      {/* Footer */}
      <div className={`border-t px-4 py-1.5 text-[10px] font-bold relative z-10 flex justify-between items-center transition-colors ${currentRelation !== 'author' ? 'bg-red-900/10 border-red-900/50' : 'bg-[#111] border-[#222]'}`}>
        <div className="flex gap-4">
            <span className="flex items-center gap-2 text-gray-500">
                SPOOKY_SYNC
            </span>
        </div>
        <div className="flex gap-4 text-gray-500">
             <span>Ln {lineCount}, Col {code.split("\n").pop()?.length}</span>
             <span>UTF-8</span>
             <span>TypeScript Solid.js</span>
        </div>
      </div>
    </div>
  );
};