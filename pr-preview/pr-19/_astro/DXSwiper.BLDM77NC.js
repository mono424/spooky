import{j as e}from"./jsx-runtime.D_zvdyIk.js";import{r as c}from"./index.DPYYMTZ4.js";import"./_commonjsHelpers.CqkleIqs.js";const g={TABLES:[{label:"user",kind:"table",type:"SCHEMAFULL",doc:"System users and authentication details."},{label:"thread",kind:"table",type:"SCHEMAFULL",doc:"Root collection for discussion threads. Contains title, body, and author references."},{label:"notification",kind:"table",type:"SCHEMAFULL",doc:"User activity alerts."}],RELATIONS:[{label:"author",kind:"relation",type:"record<user>",doc:"The creator of the thread."},{label:"comments",kind:"relation",type:"array<comment>",doc:"Replies to this thread."},{label:"categories",kind:"relation",type:"array<category>",doc:"Tags associated with thread."}],ORDER_FIELDS:[{label:"created_at",kind:"field",type:"datetime",doc:"Creation timestamp."},{label:"updated_at",kind:"field",type:"datetime",doc:"Last modification."},{label:"title",kind:"field",type:"string",doc:"Alphabetical sort."},{label:"score",kind:"field",type:"int",doc:"Popularity metric."}],DIRECTIONS:[{label:"asc",kind:"keyword",doc:"Ascending (A-Z, 0-9)."},{label:"desc",kind:"keyword",doc:"Descending (Z-A, 9-0)."}]},M=t=>g.TABLES.find(r=>r.label===t)?{options:g.TABLES,title:"Table Schema",isReadOnly:!0}:g.RELATIONS.find(r=>r.label===t)?{options:g.RELATIONS,title:"Select Relation",isReadOnly:!1}:g.ORDER_FIELDS.find(r=>r.label===t)?{options:g.ORDER_FIELDS,title:"Order By",isReadOnly:!1}:g.DIRECTIONS.find(r=>r.label===t)?{options:g.DIRECTIONS,title:"Direction",isReadOnly:!1}:null,S={field:e.jsx("svg",{className:"w-3 h-3 text-blue-400",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:e.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:2,d:"M4 6h16M4 12h16M4 18h16"})}),table:e.jsx("svg",{className:"w-3 h-3 text-orange-400",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:e.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:2,d:"M3 10h18M3 14h18m-9-4v8m-7-4h14M2 5h20v14H2V5z"})}),relation:e.jsx("svg",{className:"w-3 h-3 text-brand-400",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:e.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:2,d:"M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"})}),keyword:e.jsx("svg",{className:"w-3 h-3 text-gray-400",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:e.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:2,d:"M7 7h.01M7 3h5c.512 0 1.024.195 1.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.994 1.994 0 013 12V7a4 4 0 014-4z"})}),error:e.jsx("svg",{className:"w-3 h-3 text-red-400",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:e.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:2,d:"M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"})})},E=({text:t,className:r,cleanText:o,onReplace:u})=>{const[m,v]=c.useState(!1),f=c.useRef(null),y=M(o);if(!y)return e.jsx("span",{className:r,dangerouslySetInnerHTML:{__html:t}});const i=y.options.find(d=>d.label===o),n=()=>{f.current&&window.clearTimeout(f.current),v(!0)},p=()=>{f.current=window.setTimeout(()=>{v(!1)},150)},x=m?"z-[9999] relative":"z-auto relative";return y.isReadOnly?e.jsxs("span",{className:`${r} inline-block ${x}`,onMouseEnter:n,onMouseLeave:p,children:[e.jsx("span",{className:"absolute inset-0 -bottom-2 bg-transparent z-10"}),e.jsx("span",{className:"cursor-help decoration-dotted underline decoration-white/30 underline-offset-4 hover:decoration-white/60 hover:text-white transition-all relative z-20",dangerouslySetInnerHTML:{__html:t}}),m&&e.jsxs("div",{className:"absolute left-0 top-full mt-1 w-[280px] bg-[#1a1a1a] border border-[#333] shadow-[0_8px_32px_rgba(0,0,0,0.9)] rounded-sm font-mono text-xs animate-in fade-in slide-in-from-top-1 duration-150 pointer-events-auto cursor-default z-30",children:[e.jsxs("div",{className:"bg-[#111] px-3 py-2 border-b border-[#222] flex items-center gap-2",children:[S[i?.kind||"table"],e.jsx("span",{className:"font-bold text-gray-200",children:o}),e.jsx("span",{className:"text-[10px] text-orange-400 bg-orange-400/10 px-1.5 rounded border border-orange-400/20 ml-auto",children:i?.type})]}),e.jsx("div",{className:"p-3 text-gray-400 leading-relaxed whitespace-normal break-words",children:i?.doc}),e.jsxs("div",{className:"bg-[#111] px-3 py-1.5 border-t border-[#222] text-[10px] text-gray-600",children:[e.jsx("span",{className:"text-blue-500",children:"★"})," Primary Key:"," ",e.jsx("span",{className:"text-gray-400",children:"id"})]})]})]}):e.jsxs("span",{className:`${r} inline-block ${x}`,onMouseEnter:n,onMouseLeave:p,children:[e.jsx("span",{className:"absolute inset-0 -bottom-2 bg-transparent z-10"}),e.jsx("span",{className:"cursor-pointer border-b border-dashed border-green-500/40 bg-green-500/5 hover:bg-green-500/10 hover:border-green-400 transition-colors rounded-sm px-0.5 relative z-20",dangerouslySetInnerHTML:{__html:t}}),m&&e.jsxs("div",{className:"absolute left-0 top-full mt-1 min-w-[240px] bg-surface-elevated border border-surface-border shadow-2xl rounded-lg font-mono text-xs animate-in fade-in zoom-in-95 duration-100 overflow-hidden pointer-events-auto z-30",children:[e.jsxs("div",{className:"bg-surface px-3 py-2 text-xs text-text-tertiary border-b border-surface-border flex justify-between items-center shrink-0",children:[e.jsx("span",{className:"font-semibold text-text-primary",children:y.title}),e.jsx("span",{className:"bg-surface-border px-1.5 py-0.5 rounded text-[10px]",children:"Tab ↹"})]}),e.jsx("div",{className:"py-1",children:y.options.map(d=>{const b=d.label===o;return e.jsxs("button",{onClick:l=>{l.stopPropagation(),u(o,d.label),v(!1)},className:`w-full text-left px-3 py-2 flex justify-between items-center group transition-colors ${b?"bg-accent-500/10 text-text-primary border-l-2 border-accent-500":"text-text-tertiary border-l-2 border-transparent hover:bg-surface-hover hover:text-text-secondary"}`,children:[e.jsxs("div",{className:"flex items-center gap-2",children:[e.jsx("span",{className:"opacity-80 shrink-0",children:S[d.kind]}),e.jsx("span",{className:b?"font-semibold":"",children:d.label})]}),d.type&&e.jsx("span",{className:`text-[10px] ml-2 ${b?"text-accent-400":"text-text-muted"}`,children:d.type})]},d.label)})}),y.options.find(d=>d.label===o)?.doc&&e.jsxs("div",{className:"bg-surface border-t border-surface-border p-2 text-text-tertiary text-[10px] leading-relaxed whitespace-normal break-words",children:[e.jsx("span",{className:"text-accent-500 font-semibold",children:"INFO: "}),y.options.find(d=>d.label===o)?.doc]})]})]})},R=({text:t})=>{const[r,o]=c.useState(!1);return e.jsxs("span",{className:`group relative cursor-pointer inline-block ${r?"z-[9999]":"z-auto"}`,onMouseEnter:()=>o(!0),onMouseLeave:()=>o(!1),children:[e.jsx("span",{className:"text-gray-300 decoration-wavy underline decoration-red-500 underline-offset-2 relative z-20",children:t}),r&&e.jsxs("div",{className:"absolute left-0 bottom-full mb-1 w-[300px] bg-surface-elevated border border-red-500/50 shadow-2xl rounded-lg font-mono text-xs z-[100] animate-in fade-in slide-in-from-bottom-1 pointer-events-auto",children:[e.jsxs("div",{className:"bg-red-500/10 px-3 py-2 border-b border-red-500/20 flex items-center gap-2 text-red-400",children:[S.error,e.jsx("span",{className:"font-semibold",children:"Property 'author' does not exist"})]}),e.jsxs("div",{className:"p-3 text-text-tertiary leading-relaxed whitespace-normal break-words",children:["Type ",e.jsx("span",{className:"text-orange-300",children:"Thread"})," has no property"," ",e.jsx("span",{className:"text-white",children:"author"}),". Did you forget to include"," ",e.jsx("span",{className:"bg-[#222] px-1 text-blue-300",children:'.related("author")'})," in your query?"]})]})]})},D=({initialCode:t,filename:r})=>{const[o,u]=c.useState(t),[m,v]=c.useState("comments"),f=(n,p)=>{u(x=>x.replace(`"${n}"`,`"${p}"`)),g.RELATIONS.find(x=>x.label===n)&&v(p)},y=n=>n.split(/(".*?"|\/\/.*$)/gm).map((x,d)=>{if(x.startsWith('"')){const l=x.slice(1,-1),a=x.replace(/</g,"&lt;").replace(/>/g,"&gt;");return e.jsx(E,{text:a,className:"text-orange-300",cleanText:l,onReplace:f},d)}if(x.startsWith("//"))return e.jsx("span",{className:"text-gray-500 italic",children:x},d);const b=x.split(/(thread\.author\.username|\b(?:useQuery|db|query|related|orderBy|limit|build|import|export|from|const|return|function|default|type)\b|[{}[\]().,;]|\s+)/g);return e.jsx("span",{children:b.map((l,a)=>l?l==="thread.author.username"?m!=="author"?e.jsx(R,{text:"thread.author.username"},a):e.jsxs("span",{children:[e.jsx("span",{className:"text-gray-300",children:"thread"}),e.jsx("span",{className:"text-gray-500",children:"."}),e.jsx("span",{className:"text-gray-300",children:"author"}),e.jsx("span",{className:"text-gray-500",children:"."}),e.jsx("span",{className:"text-gray-300",children:"username"})]},a):l.match(/\b(import|export|from|const|return|function|default|type)\b/)?e.jsx("span",{className:"text-purple-400 font-bold",children:l},a):l.match(/\b(useQuery|db|query|related|orderBy|limit|build)\b/)?e.jsx("span",{className:"text-blue-400",children:l},a):l.match(/<[A-Z][a-zA-Z]*|<div|<span|<p\b/)||l.match(/<\/[A-Za-z]+>/)?e.jsx("span",{className:"text-yellow-300",children:l},a):l.match(/[{}[\]().,;]/)||l.match(/[<>]/)?e.jsx("span",{className:"text-gray-500",children:l},a):l:null)},d)}),i=o.split(`
`).length;return e.jsxs("div",{className:"border border-surface-border bg-surface shadow-xl flex flex-col font-mono text-sm relative overflow-visible group rounded-xl min-h-[320px]",children:[e.jsxs("div",{className:"flex justify-between items-center bg-surface-elevated border-b border-surface-border px-4 py-3 text-xs select-none rounded-t-xl",children:[e.jsxs("div",{className:"flex items-center gap-3",children:[e.jsxs("div",{className:"flex gap-1.5",children:[e.jsx("div",{className:"w-3 h-3 rounded-full bg-red-500/50"}),e.jsx("div",{className:"w-3 h-3 rounded-full bg-yellow-500/50"}),e.jsx("div",{className:"w-3 h-3 rounded-full bg-accent-500/50"})]}),e.jsxs("div",{className:"flex items-center gap-2 text-text-tertiary",children:[e.jsx("span",{className:"font-semibold text-brand-400",children:"TSX"}),e.jsx("span",{children:r||"Untitled"})]})]}),e.jsx("div",{className:`text-xs font-semibold px-2 py-1 rounded transition-colors ${m!=="author"?"bg-red-500/10 text-red-400 border border-red-500/30":"text-transparent"}`,children:m!=="author"?"1 Error":""})]}),e.jsx("div",{className:"relative flex-1 bg-surface overflow-visible min-h-0 font-mono text-sm leading-relaxed",children:o.split(`
`).map((n,p)=>e.jsxs("div",{className:`flex ${p===0?"pt-4":""} ${p===o.split(`
`).length-1?"pb-4":""}`,children:[e.jsx("div",{className:"bg-surface-elevated border-r border-surface-border text-text-muted text-right pr-3 pl-2 select-none w-12 shrink-0",children:p+1}),e.jsx("div",{className:"flex-1 px-4 whitespace-pre-wrap break-words text-text-secondary",children:n.length===0?e.jsx("br",{}):y(n)})]},p))}),e.jsxs("div",{className:`border-t px-4 py-2 text-xs font-semibold relative z-10 flex justify-between items-center transition-colors rounded-b-xl ${m!=="author"?"bg-red-500/10 border-red-500/30":"bg-surface-elevated border-surface-border"}`,children:[e.jsx("div",{className:"flex gap-4",children:e.jsx("span",{className:"flex items-center gap-2 text-gray-500",children:"SPOOKY_SYNC"})}),e.jsxs("div",{className:"flex gap-4 text-gray-500",children:[e.jsxs("span",{children:["Ln ",i,", Col ",o.split(`
`).pop()?.length]}),e.jsx("span",{children:"UTF-8"}),e.jsx("span",{children:"TypeScript Solid.js"})]})]})]})},q=`import { useQuery } from "@spooky/client-solid";
import { db } from "../db";

const ThreadList = () => {
  const result = useQuery(db, () =>
    db.query("thread")
      .related("comments")
      .orderBy("created_at", "desc")
      .limit(10)
      .build()
  );

  return (
    <For each={result.data()}>
      {(thread) => (
        <div>{thread.title} by {thread.author.username}</div>
      )}
    </For>
  );
};`,A=()=>e.jsxs("div",{className:"grid grid-cols-1 lg:grid-cols-2 gap-16 items-start",children:[e.jsx("div",{className:"order-2 lg:order-1",children:e.jsx(D,{filename:"ThreadList.tsx",initialCode:q})}),e.jsxs("div",{className:"order-1 lg:order-2 space-y-6",children:[e.jsx("div",{children:e.jsxs("div",{className:"border border-surface-border bg-surface rounded-xl p-6",children:[e.jsxs("div",{className:"flex items-center gap-3 mb-4",children:[e.jsx("div",{className:"w-10 h-10 bg-brand-500/10 rounded-lg flex items-center justify-center text-brand-500",children:e.jsxs("svg",{xmlns:"http://www.w3.org/2000/svg",width:"20",height:"20",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z"}),e.jsx("path",{d:"M5 3v4"}),e.jsx("path",{d:"M9 3v4"}),e.jsx("path",{d:"M3 5h4"}),e.jsx("path",{d:"M3 9h4"})]})}),e.jsx("div",{children:e.jsx("div",{className:"text-lg font-semibold text-text-primary",children:"Context-Aware Autocomplete"})})]}),e.jsx("p",{className:"text-body-sm text-text-tertiary leading-relaxed",children:"Context-aware autocomplete knows your schema inside out. It suggests sort fields, relations, and filter operations appropriate for your current query context."})]})}),e.jsx("div",{children:e.jsxs("div",{className:"border border-surface-border bg-surface rounded-xl p-6",children:[e.jsxs("div",{className:"flex items-center gap-3 mb-4",children:[e.jsx("div",{className:"w-10 h-10 bg-accent-500/10 rounded-lg flex items-center justify-center text-accent-500",children:e.jsxs("svg",{xmlns:"http://www.w3.org/2000/svg",width:"20",height:"20",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"M15 14c.2-1 .7-1.7 1.5-2.5 1-1 1.5-2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5"}),e.jsx("path",{d:"M9 18h6"}),e.jsx("path",{d:"M10 22h4"})]})}),e.jsx("div",{children:e.jsx("div",{className:"text-lg font-semibold text-text-primary",children:"Real-Time Feedback"})})]}),e.jsx("p",{className:"text-body-sm text-text-tertiary leading-relaxed",children:"Your queries are continuously validated against your schema. Type mismatches, invalid fields, and logic errors are detected instantly and highlighted directly inside the code editor, ensuring your code is correct before execution. Try yourself and fix the type error on the left."})]})})]})]}),O=`
.browser-window {
  width: 100%;
  height: 550px;
  background: #1e1e1e;
  border-radius: 8px;
  box-shadow: 0 20px 50px rgba(0,0,0,0.5);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid #3e3e42;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}

.browser-header {
  background: #2d2d2d;
  padding: 8px 12px;
  display: flex;
  align-items: center;
  gap: 16px;
  border-bottom: 1px solid #1a1a1a;
}

.window-controls {
  display: flex;
  gap: 8px;
}

.control-dot {
  width: 12px;
  height: 12px;
  border-radius: 50%;
}
.control-dot.red { background: #ff5f56; }
.control-dot.yellow { background: #ffbd2e; }
.control-dot.green { background: #27c93f; }

.url-bar {
  flex: 1;
  background: #1a1a1a;
  border-radius: 4px;
  padding: 4px 12px;
  font-size: 12px;
  color: #999;
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid #3e3e42;
}

.url-bar:hover {
  background: #252525;
  border-color: #555;
  color: #ccc;
}

.browser-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
}

.app-viewport {
  flex: 1;
  background: #111;
  position: relative;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}

.app-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: #444;
}

.devtools-container {
  height: 320px;
  border-top: 1px solid #3e3e42;
  display: flex;
  flex-direction: column;
}
`,I=`
/* Chrome DevTools Design System - Material Design 3 */
.devtools-root {
  /* Base colors - Dark theme (default) */
  --sys-color-cdt-base-container: #1e1e1e;
  --sys-color-cdt-base-container-high: #252526;
  --sys-color-cdt-base-container-highest: #2d2d30;

  /* Surface colors */
  --sys-color-surface: #252526;
  --sys-color-surface-container: #2d2d30;
  --sys-color-surface-container-low: #1e1e1e;
  --sys-color-surface-container-high: #3e3e42;
  --sys-color-surface-container-highest: #454545;

  /* On-surface colors */
  --sys-color-on-surface: #cccccc;
  --sys-color-on-surface-subtle: #858585;
  --sys-color-on-surface-variant: #9cdcfe;

  /* Primary colors */
  --sys-color-primary: #4fc3f7;
  --sys-color-primary-bright: #61dafb;
  --sys-color-primary-container: rgba(79, 195, 247, 0.1);
  --sys-color-on-primary: #ffffff;

  /* State colors */
  --sys-color-state-hover: rgba(255, 255, 255, 0.06);
  --sys-color-state-hover-on-subtle: rgba(255, 255, 255, 0.06);
  --sys-color-state-focus: rgba(255, 255, 255, 0.12);
  --sys-color-state-pressed: rgba(255, 255, 255, 0.18);
  --sys-color-state-selected: rgba(79, 195, 247, 0.15);
  --sys-color-state-disabled: rgba(255, 255, 255, 0.03);

  /* Divider and border */
  --sys-color-divider: #3e3e42;
  --sys-color-outline: #3e3e42;
  --sys-color-outline-variant: #2d2d30;

  /* Status colors */
  --sys-color-state-on: #4caf50;
  --sys-color-state-off: #858585;
  --sys-color-error: #f48fb1;
  --sys-color-warning: #ffb74d;
  --sys-color-info: #64b5f6;

  /* Text colors */
  --sys-color-text-primary: #cccccc;
  --sys-color-text-secondary: #858585;
  --sys-color-text-disabled: #5a5a5a;

  /* Typography */
  --sys-typescale-body-font:
    'Segoe UI', system-ui, -apple-system, 'SF Pro Display', 'Roboto', sans-serif;
  --sys-typescale-body-size: 12px;
  --sys-typescale-body-weight: 400;
  --sys-typescale-body-line-height: 18px;

  --sys-typescale-label-font:
    'Segoe UI', system-ui, -apple-system, 'SF Pro Display', 'Roboto', sans-serif;
  --sys-typescale-label-size: 11px;
  --sys-typescale-label-weight: 500;
  --sys-typescale-label-line-height: 16px;

  --sys-typescale-title-font:
    'Segoe UI', system-ui, -apple-system, 'SF Pro Display', 'Roboto', sans-serif;
  --sys-typescale-title-size: 13px;
  --sys-typescale-title-weight: 500;
  --sys-typescale-title-line-height: 20px;

  --sys-typescale-monospace-font: 'Consolas', 'Monaco', 'Courier New', monospace;
  --sys-typescale-monospace-size: 11px;
  --sys-typescale-monospace-line-height: 16px;

  /* Spacing */
  --sys-spacing-1: 4px;
  --sys-spacing-2: 8px;
  --sys-spacing-3: 12px;
  --sys-spacing-4: 16px;
  --sys-spacing-5: 20px;
  --sys-spacing-6: 24px;

  /* Border radius */
  --sys-radius-xs: 2px;
  --sys-radius-sm: 4px;
  --sys-radius-md: 6px;
  --sys-radius-lg: 8px;

  /* Shadows */
  --sys-shadow-1: 0 1px 2px rgba(0, 0, 0, 0.3);
  --sys-shadow-2: 0 2px 4px rgba(0, 0, 0, 0.3);
  --sys-shadow-3: 0 4px 8px rgba(0, 0, 0, 0.3);

  /* Transitions */
  --sys-transition-fast: 100ms cubic-bezier(0.4, 0, 0.2, 1);
  --sys-transition-base: 200ms cubic-bezier(0.4, 0, 0.2, 1);
  --sys-transition-slow: 300ms cubic-bezier(0.4, 0, 0.2, 1);

  font-family: var(--sys-typescale-body-font);
  font-size: var(--sys-typescale-body-size);
  font-weight: var(--sys-typescale-body-weight);
  line-height: var(--sys-typescale-body-line-height);
  background: var(--sys-color-cdt-base-container);
  color: var(--sys-color-on-surface);
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  text-align: left;
}

.devtools-root * {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

/* Header/Toolbar */
.devtools-root .tabs {
  display: flex;
  background: var(--sys-color-surface);
  border-bottom: 1px solid var(--sys-color-divider);
  flex-shrink: 0;
}

.devtools-root .toolbar-group {
  display: flex;
  align-items: center;
  gap: var(--sys-spacing-1);
}

.devtools-root .toolbar-group + .toolbar-group {
  margin-left: var(--sys-spacing-2);
  padding-left: var(--sys-spacing-2);
  border-left: 1px solid var(--sys-color-divider);
}

.devtools-root .toolbar-group-right {
  display: flex;
  align-items: center;
  gap: var(--sys-spacing-1);
  margin-left: auto;
  flex-grow: 1;
  justify-content: flex-end;
  padding-right: var(--sys-spacing-2);
}

/* Buttons */
.devtools-root .btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: var(--sys-spacing-1) var(--sys-spacing-2);
  min-height: 20px;
  background: transparent;
  color: var(--sys-color-on-surface);
  border: none;
  border-radius: var(--sys-radius-xs);
  cursor: pointer;
  font-family: var(--sys-typescale-label-font);
  font-size: var(--sys-typescale-label-size);
  font-weight: var(--sys-typescale-label-weight);
  line-height: var(--sys-typescale-label-line-height);
  transition: background-color var(--sys-transition-fast);
  user-select: none;
}

.devtools-root .btn:hover {
  background: var(--sys-color-state-hover);
}

/* Status indicator */
.devtools-root .status-indicator {
  display: inline-flex;
  align-items: center;
  gap: var(--sys-spacing-1);
  padding: var(--sys-spacing-1) var(--sys-spacing-2);
  font-size: var(--sys-typescale-label-size);
  line-height: var(--sys-typescale-label-line-height);
}

.devtools-root .status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.devtools-root .status-dot.active {
  background: var(--sys-color-state-on);
  box-shadow: 0 0 4px var(--sys-color-state-on);
}

/* Tabs */
.devtools-root .tab-btn {
  position: relative;
  padding: var(--sys-spacing-1) var(--sys-spacing-2);
  background: transparent;
  color: var(--sys-color-on-surface-subtle);
  border: none;
  border-bottom: 2px solid transparent;
  cursor: pointer;
  font-family: var(--sys-typescale-label-font);
  font-size: var(--sys-typescale-label-size);
  font-weight: var(--sys-typescale-label-weight);
  line-height: var(--sys-typescale-label-line-height);
  transition: color var(--sys-transition-fast), background-color var(--sys-transition-fast);
  user-select: none;
}

.devtools-root .tab-btn:hover {
  background: var(--sys-color-state-hover);
  color: var(--sys-color-on-surface);
}

.devtools-root .tab-btn.active {
  color: var(--sys-color-primary-bright);
  border-bottom-color: var(--sys-color-primary);
}

/* Content area */
.devtools-root .content {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  background: var(--sys-color-cdt-base-container);
}

.devtools-root .tab-content {
  display: none;
  flex: 1;
  overflow: hidden;
  flex-direction: column;
}

.devtools-root .tab-content.active {
  display: flex;
}

/* Events tab */
.devtools-root .events-container {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.devtools-root .events-header {
  padding: 6px 8px;
  border-bottom: 1px solid var(--sys-color-divider);
  flex-shrink: 0;
  min-height: 24px;
  display: flex;
  align-items: center;
}

.devtools-root .events-header h2 {
  font-family: var(--sys-typescale-label-font);
  font-size: var(--sys-typescale-label-size);
  font-weight: var(--sys-typescale-label-weight);
  line-height: var(--sys-typescale-label-line-height);
  color: var(--sys-color-on-surface);
  margin: 0;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.devtools-root .events-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}

.devtools-root .event-item {
  background: var(--sys-color-surface);
  border-radius: var(--sys-radius-xs);
  padding: 6px 8px;
  margin-bottom: 4px;
  border-left: 2px solid var(--sys-color-divider);
  transition: background-color var(--sys-transition-fast), border-color var(--sys-transition-fast);
}

.devtools-root .event-item:hover {
  background: var(--sys-color-state-hover-on-subtle);
}

.devtools-root .event-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--sys-spacing-1);
}

.devtools-root .event-type {
  font-family: var(--sys-typescale-label-font);
  font-size: var(--sys-typescale-label-size);
  font-weight: var(--sys-typescale-label-weight);
  color: var(--sys-color-on-surface);
}

.devtools-root .event-time {
  font-family: var(--sys-typescale-monospace-font);
  font-size: var(--sys-typescale-monospace-size);
  color: var(--sys-color-on-surface-subtle);
}

.devtools-root .event-payload {
  background: var(--sys-color-cdt-base-container);
  border-radius: var(--sys-radius-xs);
  padding: var(--sys-spacing-2);
  margin-top: var(--sys-spacing-1);
  border: 1px solid var(--sys-color-divider);
  overflow-x: auto;
}

.devtools-root .event-payload pre {
  margin: 0;
  white-space: pre-wrap;
  word-wrap: break-word;
  font-family: var(--sys-typescale-monospace-font);
  font-size: var(--sys-typescale-monospace-size);
  line-height: var(--sys-typescale-monospace-line-height);
  color: var(--sys-color-on-surface-subtle);
}

/* Queries tab */
.devtools-root .queries-container {
  display: grid;
  grid-template-columns: 280px 1fr;
  height: 100%;
  overflow: hidden;
}

.devtools-root .queries-list {
  border-right: 1px solid var(--sys-color-divider);
  overflow: hidden;
  background: var(--sys-color-surface);
  display: flex;
  flex-direction: column;
}

.devtools-root .queries-list-content {
  flex: 1;
  overflow-y: auto;
  padding: 0;
  margin: 0;
}

.devtools-root .queries-header {
  padding: 6px 8px;
  border-bottom: 1px solid var(--sys-color-divider);
  background: var(--sys-color-surface);
  min-height: 24px;
  display: flex;
  align-items: center;
}

.devtools-root .queries-header h2 {
    font-family: var(--sys-typescale-label-font);
    font-size: var(--sys-typescale-label-size);
    font-weight: var(--sys-typescale-label-weight);
    line-height: var(--sys-typescale-label-line-height);
    color: var(--sys-color-on-surface);
    margin: 0;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

.devtools-root .query-item {
  padding: 4px 8px;
  cursor: pointer;
  border: 2px solid transparent;
  transition: background-color var(--sys-transition-fast);
  list-style: none;
  background: transparent;
}

.devtools-root .query-item:hover:not(.selected) {
  background: var(--sys-color-state-hover-on-subtle) !important;
}

.devtools-root .query-item.selected {
  background: var(--sys-color-tonal-container, rgba(79, 195, 247, 0.2)) !important;
}

.devtools-root .query-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--sys-spacing-1);
}

.devtools-root .query-hash {
  font-family: var(--sys-typescale-label-font);
  font-size: var(--sys-typescale-label-size);
  font-weight: var(--sys-typescale-label-weight);
  color: var(--sys-color-on-surface);
}

.devtools-root .query-status {
  padding: 2px var(--sys-spacing-1);
  border-radius: var(--sys-radius-xs);
  font-family: var(--sys-typescale-label-font);
  font-size: 9px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  line-height: 12px;
}

.devtools-root .status-active {
  background: rgba(76, 175, 80, 0.2);
  color: var(--sys-color-state-on);
}

.devtools-root .status-idle {
    background: rgba(133, 133, 133, 0.2);
    color: #858585;
}

.devtools-root .query-meta {
  font-family: var(--sys-typescale-monospace-font);
  font-size: var(--sys-typescale-monospace-size);
  color: var(--sys-color-on-surface-subtle);
  margin-top: var(--sys-spacing-1);
}

.devtools-root .query-preview {
  font-family: var(--sys-typescale-monospace-font);
  font-size: 10px;
  color: var(--sys-color-on-surface-subtle);
  margin-top: var(--sys-spacing-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.devtools-root .query-detail {
  overflow-y: auto;
  background: var(--sys-color-cdt-base-container);
  padding: 8px;
}

.devtools-root .detail-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: 8px;
  margin-bottom: 8px;
  border-bottom: 1px solid var(--sys-color-divider);
}

.devtools-root .detail-header h3 {
  font-family: var(--sys-typescale-title-font);
  font-size: var(--sys-typescale-title-size);
  font-weight: var(--sys-typescale-title-weight);
  color: var(--sys-color-primary-bright);
}

.devtools-root .detail-section {
  margin-bottom: 12px;
}

.devtools-root .detail-label {
  font-family: var(--sys-typescale-label-font);
  font-size: 10px;
  font-weight: var(--sys-typescale-label-weight);
  color: var(--sys-color-on-surface-subtle);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 4px;
}

.devtools-root .detail-value {
  font-family: var(--sys-typescale-body-font);
  font-size: var(--sys-typescale-body-size);
  color: var(--sys-color-on-surface);
}

.devtools-root .detail-value.mono {
  font-family: var(--sys-typescale-monospace-font);
  font-size: var(--sys-typescale-monospace-size);
  color: var(--sys-color-primary);
}

.devtools-root .query-code {
  background: var(--sys-color-cdt-base-container);
  border-radius: var(--sys-radius-xs);
  padding: 6px 8px;
  margin: 0;
  overflow-x: auto;
  font-family: var(--sys-typescale-monospace-font);
  font-size: var(--sys-typescale-monospace-size);
  line-height: var(--sys-typescale-monospace-line-height);
  color: var(--sys-color-on-surface);
  border: 1px solid var(--sys-color-divider);
  white-space: pre-wrap;
  word-wrap: break-word;
}

/* Database tab */
.devtools-root .database-container {
  display: grid;
  grid-template-columns: 200px 1fr;
  height: 100%;
  overflow: hidden;
}

.devtools-root .database-tables {
  border-right: 1px solid var(--sys-color-divider);
  overflow: hidden;
  background: var(--sys-color-surface);
  display: flex;
  flex-direction: column;
}

.devtools-root .tables-list {
  flex: 1;
  overflow-y: auto;
}

.devtools-root .table-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 12px;
  cursor: pointer;
  border-left: 2px solid transparent;
  width: 100%;
}

.devtools-root .table-item:hover {
    background: var(--sys-color-state-hover-on-subtle);
}

.devtools-root .table-item.selected {
    background: var(--sys-color-tonal-container, rgba(79, 195, 247, 0.2));
    border-left-color: var(--sys-color-primary);
}

.devtools-root .table-name {
    font-family: var(--sys-typescale-monospace-font);
    font-size: var(--sys-typescale-monospace-size);
    color: var(--sys-color-on-surface);
}

.devtools-root .table-count {
    font-family: var(--sys-typescale-label-font);
    font-size: 10px;
    background: var(--sys-color-surface-container-high);
    padding: 1px 6px;
    border-radius: 8px;
    color: var(--sys-color-on-surface-subtle);
}

.devtools-root .data-grid {
    flex: 1;
    overflow: auto;
    font-family: var(--sys-typescale-monospace-font);
    font-size: var(--sys-typescale-monospace-size);
}
.devtools-root table {
    width: 100%;
    border-collapse: collapse;
}
.devtools-root th, .devtools-root td {
    padding: 6px 12px;
    border-bottom: 1px solid var(--sys-color-divider);
    text-align: left;
}
.devtools-root th {
    color: var(--sys-color-on-surface-subtle);
    font-weight: normal;
    background: var(--sys-color-surface);
    position: sticky;
    top: 0;
}
.devtools-root td {
    color: var(--sys-color-on-surface);
    white-space: nowrap;
}
.devtools-root tr:hover td {
    background: var(--sys-color-state-hover);
}
.devtools-root .text-primary { color: var(--sys-color-primary); }
.devtools-root .text-string { color: #ce9178; }
.devtools-root .text-number { color: #b5cea8; }
`,B=[{type:"info",timestamp:1715011321152,data:{msg:"Spooky Sidecar initialized"}},{type:"info",timestamp:1715011321340,data:{msg:"Loading dbsp_worker.wasm... (2.1MB)"}},{type:"warn",timestamp:1715011322005,data:{msg:"State rehydration took 115ms",hints:["optimize-query"]}},{type:"info",timestamp:1715011325112,data:{msg:'Registered incantation "thread_list_view"'}}],L=[{queryHash:"0x8f2a9c",status:"active",createdAt:1715011325112,lastUpdate:1715011335112,updateCount:12,dataSize:1024,query:"SELECT * FROM thread ORDER BY created_at DESC",variables:{limit:10},data:[{id:"thread:1",title:"Why Rust?"}]},{queryHash:"0x99b1d4",status:"idle",createdAt:1715011326e3,lastUpdate:171501133e4,updateCount:5,dataSize:256,query:"SELECT count() FROM notifications",variables:{},data:{count:3}}],P=[{name:"user",count:12},{name:"thread",count:45},{name:"comment",count:182}],_=[{id:"thread:8x92m",title:"Why Rust is the future",author:"user:khadim",created:"2024-05-01"},{id:"thread:p09s1",title:"Local-first architecture",author:"user:sarah",created:"2024-05-02"},{id:"thread:7d66s",title:"Spooky vs ElectricSQL",author:"user:alex",created:"2024-05-03"}],$=()=>{const[t,r]=c.useState("events"),[o,u]=c.useState("0x8f2a9c"),[m,v]=c.useState(0),[f,y]=c.useState(0),i=["events","queries","database","auth"],n=L.find(s=>s.queryHash===o),p=s=>{v(s.targetTouches[0].clientX)},x=s=>{y(s.targetTouches[0].clientX)},d=()=>{if(!m||!f)return;const s=m-f,j=s>50,k=s<-50,w=i.indexOf(t);j&&w<i.length-1&&r(i[w+1]),k&&w>0&&r(i[w-1]),v(0),y(0)},[b,l]=c.useState(0),[a,h]=c.useState(!1),z=s=>{l(s.clientX),h(!0)},C=s=>{},T=s=>{if(!a)return;const j=b-s.clientX,k=j>80,w=j<-80,N=i.indexOf(t);k&&N<i.length-1&&r(i[N+1]),w&&N>0&&r(i[N-1]),h(!1)};return e.jsxs(e.Fragment,{children:[e.jsx("style",{children:O}),e.jsx("style",{children:I}),e.jsxs("div",{className:"browser-window",children:[e.jsxs("div",{className:"browser-header",children:[e.jsxs("div",{className:"window-controls",children:[e.jsx("div",{className:"control-dot red"}),e.jsx("div",{className:"control-dot yellow"}),e.jsx("div",{className:"control-dot green"})]}),e.jsxs("div",{className:"url-bar",children:[e.jsxs("svg",{width:"12",height:"12",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("rect",{x:"3",y:"11",width:"18",height:"11",rx:"2",ry:"2"}),e.jsx("path",{d:"M7 11V7a5 5 0 0 1 10 0v4"})]}),e.jsx("span",{children:"localhost:5173"})]})]}),e.jsxs("div",{className:"browser-body",children:[e.jsx("div",{className:"app-viewport",children:e.jsxs("div",{className:"app-placeholder",children:[e.jsxs("svg",{style:{opacity:.05,marginBottom:20},width:"100",height:"100",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"1",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"M12 2L2 7l10 5 10-5-10-5z"}),e.jsx("path",{d:"M2 17l10 5 10-5"}),e.jsx("path",{d:"M2 12l10 5 10-5"})]}),e.jsx("h1",{style:{fontSize:24,fontWeight:"bold",marginBottom:8},children:"My Spooky App"}),e.jsx("p",{style:{fontSize:14,opacity:.6},children:"Syncing active..."})]})}),e.jsx("div",{className:"devtools-container",children:e.jsxs("div",{className:"devtools-root",children:[e.jsxs("div",{className:"tabs",children:[e.jsx("div",{className:"toolbar-group",children:e.jsx("div",{className:"status-indicator",children:e.jsx("span",{className:"status-dot active"})})}),i.map(s=>e.jsx("button",{className:`tab-btn ${t===s?"active":""}`,onClick:()=>r(s),children:s.charAt(0).toUpperCase()+s.slice(1)},s)),e.jsxs("div",{className:"toolbar-group-right",children:[e.jsx("button",{className:"btn",children:"Refresh"}),e.jsx("button",{className:"btn",children:"Clear Events"})]})]}),e.jsxs("div",{className:"content",onTouchStart:p,onTouchMove:x,onTouchEnd:d,onPointerDown:z,onPointerMove:C,onPointerUp:T,style:{touchAction:"pan-y",cursor:a?"grabbing":"grab"},children:[e.jsx("div",{className:`tab-content ${t==="events"?"active":""}`,children:e.jsxs("div",{className:"events-container",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Events History"})}),e.jsx("div",{className:"events-list",children:B.map((s,j)=>e.jsxs("div",{className:"event-item",children:[e.jsxs("div",{className:"event-header",children:[e.jsx("span",{className:"event-type",children:s.type}),e.jsxs("span",{className:"event-time",children:[new Date(s.timestamp).toLocaleTimeString([],{hour12:!1}),".",String(s.timestamp%1e3).padStart(3,"0")]})]}),s.data&&e.jsx("div",{className:"event-payload",children:e.jsx("pre",{children:JSON.stringify(s.data,null,2)})})]},j))})]})}),e.jsx("div",{className:`tab-content ${t==="queries"?"active":""}`,children:e.jsxs("div",{className:"queries-container",children:[e.jsxs("div",{className:"queries-list",children:[e.jsx("div",{className:"queries-header",children:e.jsx("h2",{children:"Active Queries"})}),e.jsx("div",{className:"queries-list-content",children:L.map(s=>e.jsxs("div",{className:`query-item ${o===s.queryHash?"selected":""}`,onClick:()=>u(s.queryHash),children:[e.jsxs("div",{className:"query-header",children:[e.jsxs("span",{className:"query-hash",children:["#",s.queryHash]}),e.jsx("span",{className:`query-status status-${s.status}`,children:s.status})]}),e.jsxs("div",{className:"query-meta",children:["Updates: ",s.updateCount," | Size: ",s.dataSize,"B"]}),e.jsxs("div",{className:"query-preview",children:[s.query.substring(0,30),"..."]})]},s.queryHash))})]}),n?e.jsxs("div",{className:"query-detail",children:[e.jsxs("div",{className:"detail-header",children:[e.jsxs("h3",{children:["Query #",n.queryHash]}),e.jsx("span",{className:`query-status status-${n.status}`,children:n.status})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Created"}),e.jsx("div",{className:"detail-value",children:new Date(n.createdAt).toLocaleTimeString()})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Update Count"}),e.jsx("div",{className:"detail-value mono",children:n.updateCount})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Query"}),e.jsx("pre",{className:"query-code",children:n.query})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Variables"}),e.jsx("pre",{className:"query-code",children:JSON.stringify(n.variables,null,2)})]})]}):e.jsx("div",{className:"query-detail"})]})}),e.jsx("div",{className:`tab-content ${t==="database"?"active":""}`,children:e.jsxs("div",{className:"database-container",children:[e.jsxs("div",{className:"database-tables",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Tables"})}),e.jsx("div",{className:"tables-list",children:P.map(s=>e.jsxs("div",{className:`table-item ${s.name==="thread"?"selected":""}`,children:[e.jsx("span",{className:"table-name",children:s.name}),e.jsx("span",{className:"table-count",children:s.count})]},s.name))})]}),e.jsx("div",{className:"data-grid",children:e.jsxs("table",{children:[e.jsx("thead",{children:e.jsxs("tr",{children:[e.jsx("th",{children:"id"}),e.jsx("th",{children:"title"}),e.jsx("th",{children:"author"}),e.jsx("th",{children:"created"})]})}),e.jsx("tbody",{children:_.map(s=>e.jsxs("tr",{children:[e.jsx("td",{className:"text-primary",children:s.id}),e.jsxs("td",{className:"text-string",children:['"',s.title,'"']}),e.jsx("td",{className:"text-primary",children:s.author}),e.jsx("td",{className:"text-number",children:s.created})]},s.id))})]})})]})}),e.jsx("div",{className:`tab-content ${t==="auth"?"active":""}`,children:e.jsxs("div",{className:"events-container",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Authentication"})}),e.jsxs("div",{style:{padding:20,color:"#858585",fontFamily:"var(--sys-typescale-body-font)"},children:["User authenticated: ",e.jsx("span",{style:{color:"#4fc3f7"},children:"user:khadim"})]})]})})]})]})})]})]})]})},H=()=>e.jsxs("div",{className:"grid grid-cols-1 lg:grid-cols-2 gap-16 items-start",children:[e.jsx("div",{className:"shadow-2xl",children:e.jsx($,{})}),e.jsxs("div",{className:"space-y-8",children:[e.jsxs("div",{children:[e.jsx("div",{className:"inline-block border border-brand-500/30 bg-brand-500/10 px-3 py-1.5 rounded-lg text-xs font-medium text-brand-400 mb-4",children:"DevTools"}),e.jsx("h3",{className:"text-2xl font-bold text-text-primary mb-4",children:"Effortlessly Transparent"}),e.jsx("p",{className:"text-text-tertiary text-body leading-relaxed",children:"Stop guessing what your application is doing. Spooky provides a complete suite of tools embedded directly in your browser."})]}),e.jsxs("div",{className:"space-y-6",children:[e.jsxs("div",{className:"group",children:[e.jsxs("h4",{className:"text-text-primary font-semibold mb-2 group-hover:text-brand-400 transition-colors flex items-center gap-2",children:[e.jsxs("svg",{xmlns:"http://www.w3.org/2000/svg",width:"16",height:"16",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"M21 11V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h6"}),e.jsx("path",{d:"m12 12 4 10 1.7-4.3L22 16Z"})]}),"Live Inspection"]}),e.jsx("p",{className:"text-text-tertiary leading-relaxed",children:"View your local database state in real-time as it changes. No more console.logging state."})]}),e.jsxs("div",{className:"group",children:[e.jsxs("h4",{className:"text-text-primary font-semibold mb-2 group-hover:text-accent-400 transition-colors flex items-center gap-2",children:[e.jsxs("svg",{xmlns:"http://www.w3.org/2000/svg",width:"16",height:"16",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"M12 2v4"}),e.jsx("path",{d:"m16.2 7.8 2.9-2.9"}),e.jsx("path",{d:"M18 12h4"}),e.jsx("path",{d:"m16.2 16.2 2.9 2.9"}),e.jsx("path",{d:"M12 18v4"}),e.jsx("path",{d:"m4.9 19.1 2.9-2.9"}),e.jsx("path",{d:"M2 12h4"}),e.jsx("path",{d:"m4.9 4.9 2.9 2.9"})]}),"Query Monitor"]}),e.jsx("p",{className:"text-text-tertiary leading-relaxed",children:"Track active subscriptions, latency metrics, and data transfer sizes for optimization."})]}),e.jsxs("div",{className:"group",children:[e.jsxs("h4",{className:"text-text-primary font-semibold mb-2 group-hover:text-brand-400 transition-colors flex items-center gap-2",children:[e.jsxs("svg",{xmlns:"http://www.w3.org/2000/svg",width:"16",height:"16",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("circle",{cx:"12",cy:"12",r:"10"}),e.jsx("polyline",{points:"12 6 12 12 16 14"})]}),"Events Timeline"]}),e.jsx("p",{className:"text-text-tertiary leading-relaxed",children:"Track every state change, network request, and incantation registration in a detailed, chronological timeline."})]})]})]})]}),Q=()=>{const[t,r]=c.useState(0),[o,u]=c.useState(!1),[m,v]=c.useState(0),[f,y]=c.useState(!1),i=2,n=c.useCallback(()=>{if(o||t>=i-1)return;u(!0);const a=Math.min(t+1,i-1);console.log("Moving to pane:",a),r(a),setTimeout(()=>u(!1),400)},[t,i,o]),p=c.useCallback(()=>{o||t<=0||(u(!0),r(a=>Math.max(a-1,0)),setTimeout(()=>u(!1),400))},[t,o]),x=c.useCallback(a=>{o||a===t||(u(!0),r(a),setTimeout(()=>u(!1),400))},[t,o]);c.useEffect(()=>{const a=h=>{if(!o)switch(h.key){case"ArrowLeft":h.preventDefault(),p();break;case"ArrowRight":h.preventDefault(),n();break;case"Home":h.preventDefault(),t!==0&&(u(!0),r(0),setTimeout(()=>u(!1),400));break;case"End":h.preventDefault(),t!==i-1&&(u(!0),r(i-1),setTimeout(()=>u(!1),400));break}};return window.addEventListener("keydown",a),()=>window.removeEventListener("keydown",a)},[t,i,o,n,p]);const d=a=>{v(a.clientX),y(!0)},b=a=>{if(!f||o)return;const h=a.clientX-m;Math.abs(h)>50&&(h>0&&t>0?p():h<0&&t<i-1&&n(),y(!1))},l=()=>{y(!1)};return e.jsxs("div",{role:"region","aria-label":"Developer Experience Features",className:"mt-12",children:[e.jsx("div",{className:"swiper-container",style:{position:"relative",overflow:"hidden",width:"100%"},onPointerDown:d,onPointerMove:b,onPointerUp:l,onPointerCancel:l,children:e.jsxs("div",{className:"swiper-track",style:{display:"flex",transform:`translateX(-${t*100}%)`,transition:"transform 400ms ease-in-out",willChange:"transform"},children:[e.jsx("div",{className:"swiper-slide",style:{minWidth:"100%",width:"100%",flexShrink:0},"aria-hidden":t!==0,children:e.jsx(A,{})}),e.jsx("div",{className:"swiper-slide",style:{minWidth:"100%",width:"100%",flexShrink:0},"aria-hidden":t!==1,children:e.jsx(H,{})})]})}),e.jsxs("div",{className:"sr-only",role:"status","aria-live":"polite","aria-atomic":"true",children:["Showing feature ",t+1," of ",i]}),e.jsxs("div",{className:"flex items-center justify-center gap-6 mt-12",children:[e.jsx("button",{onClick:p,disabled:t===0,"aria-label":"Previous feature",className:"swiper-arrow",children:e.jsx("svg",{xmlns:"http://www.w3.org/2000/svg",width:"20",height:"20",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:e.jsx("path",{d:"m15 18-6-6 6-6"})})}),e.jsx("div",{className:"swiper-dots",role:"tablist","aria-label":"Feature navigation",children:Array.from({length:i}).map((a,h)=>e.jsx("button",{onClick:()=>x(h),"aria-label":`Go to feature ${h+1}`,"aria-current":t===h?"true":"false",role:"tab","aria-selected":t===h,className:"swiper-dot"},h))}),e.jsx("button",{onClick:n,disabled:t===i-1,"aria-label":"Next feature",className:"swiper-arrow",children:e.jsx("svg",{xmlns:"http://www.w3.org/2000/svg",width:"20",height:"20",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:e.jsx("path",{d:"m9 18 6-6-6-6"})})})]})]})};export{Q as DXSwiper};
