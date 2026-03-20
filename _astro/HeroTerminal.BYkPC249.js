import{j as e}from"./jsx-runtime.D_zvdyIk.js";import"./index.DPYYMTZ4.js";import"./_commonjsHelpers.CqkleIqs.js";const n=()=>e.jsxs("div",{className:"hero-terminal relative w-full rounded-xl border border-white/10 bg-surface-elevated overflow-hidden transition-all duration-300",children:[e.jsx("div",{className:"terminal-chrome flex items-center justify-between bg-surface-elevated border-b border-white/10 px-4 py-3 transition-all duration-300",children:e.jsxs("div",{className:"flex items-center gap-3",children:[e.jsxs("div",{className:"traffic-lights flex gap-1.5",children:[e.jsx("span",{className:"dot w-3 h-3 rounded-full bg-white/15"}),e.jsx("span",{className:"dot w-3 h-3 rounded-full bg-white/15"}),e.jsx("span",{className:"dot w-3 h-3 rounded-full bg-white/15"})]}),e.jsx("span",{className:"terminal-title text-text-tertiary text-xs",children:"queries.ts"})]})}),e.jsx("div",{className:"terminal-content relative z-10 bg-surface p-6 transition-all duration-300 hidden lg:block",children:e.jsx("pre",{className:"font-mono text-sm text-white/60 leading-relaxed whitespace-pre-wrap break-words",children:e.jsxs("code",{children:[e.jsx("span",{className:"text-brand-400/70",children:"import"})," ","{"," useQuery ","}"," ",e.jsx("span",{className:"text-brand-400/70",children:"from"})," ",e.jsx("span",{className:"text-white/65",children:'"@spooky-sync/client-solid"'}),";",`
`,e.jsx("span",{className:"text-brand-400/70",children:"import"})," ","{"," db ","}"," ",e.jsx("span",{className:"text-brand-400/70",children:"from"})," ",e.jsx("span",{className:"text-white/65",children:'"../db"'}),";",`
`,`
`,e.jsx("span",{className:"text-brand-400/70",children:"const"})," ",e.jsx("span",{className:"text-white/85",children:"ThreadList"})," = () => ","{",`
`,"  ",e.jsx("span",{className:"text-brand-400/70",children:"const"})," result = ",e.jsx("span",{className:"text-white/85",children:"useQuery"}),"(db, () =>",`
`,"    db.",e.jsx("span",{className:"text-white/85",children:"query"}),"(",e.jsx("span",{className:"text-white/65",children:'"thread"'}),")",`
`,"      .",e.jsx("span",{className:"text-white/85",children:"related"}),"(",e.jsx("span",{className:"text-white/65",children:'"author"'}),")",`
`,"      .",e.jsx("span",{className:"text-white/85",children:"related"}),"(",e.jsx("span",{className:"text-white/65",children:'"comments"'}),")",`
`,"      .",e.jsx("span",{className:"text-white/85",children:"orderBy"}),"(",e.jsx("span",{className:"text-white/65",children:'"created_at"'}),", ",e.jsx("span",{className:"text-white/65",children:'"desc"'}),")",`
`,"      .",e.jsx("span",{className:"text-white/85",children:"limit"}),"(",e.jsx("span",{className:"text-white/65",children:"10"}),")",`
`,"      .",e.jsx("span",{className:"text-white/85",children:"build"}),"()",`
`,"  );",`
`,`
`,"  ",e.jsx("span",{className:"text-brand-400/70",children:"return"})," (",`
`,"    ",e.jsx("span",{className:"text-white/45",children:"<"}),e.jsx("span",{className:"text-white/60",children:"For"})," ",e.jsx("span",{className:"text-white/60",children:"each"}),"=",e.jsx("span",{className:"text-white/45",children:"{"}),"result.",e.jsx("span",{className:"text-white/85",children:"data"}),"()",e.jsx("span",{className:"text-white/45",children:"}"}),e.jsx("span",{className:"text-white/45",children:">"}),`
`,"      ",e.jsx("span",{className:"text-white/45",children:"{"}),"(thread) => (",`
`,"        ",e.jsx("span",{className:"text-white/45",children:"<"}),e.jsx("span",{className:"text-white/60",children:"div"}),e.jsx("span",{className:"text-white/45",children:">"}),e.jsx("span",{className:"text-white/45",children:"{"}),"thread.title",e.jsx("span",{className:"text-white/45",children:"}"}),e.jsx("span",{className:"text-white/45",children:"</"}),e.jsx("span",{className:"text-white/60",children:"div"}),e.jsx("span",{className:"text-white/45",children:">"}),`
`,"      )","}",`
`,"    ",e.jsx("span",{className:"text-white/45",children:"</"}),e.jsx("span",{className:"text-white/60",children:"For"}),e.jsx("span",{className:"text-white/45",children:">"}),`
`,"  );",`
`,"}",";"]})})}),e.jsx("div",{className:"terminal-content relative z-10 bg-surface p-4 transition-all duration-300 lg:hidden",children:e.jsx("pre",{className:"font-mono text-xs text-white/60 leading-relaxed whitespace-pre-wrap break-words",children:e.jsxs("code",{children:[e.jsx("span",{className:"text-brand-400/70",children:"const"})," result = ",e.jsx("span",{className:"text-white/85",children:"useQuery"}),"(db, () =>",`
`,"  db.",e.jsx("span",{className:"text-white/85",children:"query"}),"(",e.jsx("span",{className:"text-white/65",children:'"thread"'}),")",`
`,"    .",e.jsx("span",{className:"text-white/85",children:"related"}),"(",e.jsx("span",{className:"text-white/65",children:'"author"'}),")",`
`,"    .",e.jsx("span",{className:"text-white/85",children:"related"}),"(",e.jsx("span",{className:"text-white/65",children:'"comments"'}),")",`
`,"    .",e.jsx("span",{className:"text-white/85",children:"orderBy"}),"(",e.jsx("span",{className:"text-white/65",children:'"created_at"'}),", ",e.jsx("span",{className:"text-white/65",children:'"desc"'}),")",`
`,"    .",e.jsx("span",{className:"text-white/85",children:"limit"}),"(",e.jsx("span",{className:"text-white/65",children:"10"}),")",`
`,"    .",e.jsx("span",{className:"text-white/85",children:"build"}),"()",`
`,");"]})})})]});export{n as HeroTerminal};
