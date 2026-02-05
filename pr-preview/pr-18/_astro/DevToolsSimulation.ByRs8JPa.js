import{j as e}from"./jsx-runtime.D_zvdyIk.js";import{r}from"./index.DPYYMTZ4.js";import"./_commonjsHelpers.CqkleIqs.js";const d=`
.browser-window {
  width: 100%;
  height: 450px;
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
  height: 250px;
  border-top: 1px solid #3e3e42;
  display: flex;
  flex-direction: column;
}
`,y=`
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
`,p=[{type:"info",timestamp:1715011321152,data:{msg:"Spooky Sidecar initialized"}},{type:"info",timestamp:1715011321340,data:{msg:"Loading dbsp_worker.wasm... (2.1MB)"}},{type:"warn",timestamp:1715011322005,data:{msg:"State rehydration took 115ms",hints:["optimize-query"]}},{type:"info",timestamp:1715011325112,data:{msg:'Registered incantation "thread_list_view"'}}],l=[{queryHash:"0x8f2a9c",status:"active",createdAt:1715011325112,lastUpdate:1715011335112,updateCount:12,dataSize:1024,query:"SELECT * FROM thread ORDER BY created_at DESC",variables:{limit:10},data:[{id:"thread:1",title:"Why Rust?"}]},{queryHash:"0x99b1d4",status:"idle",createdAt:1715011326e3,lastUpdate:171501133e4,updateCount:5,dataSize:256,query:"SELECT count() FROM notifications",variables:{},data:{count:3}}],v=[{name:"user",count:12},{name:"thread",count:45},{name:"comment",count:182}],h=[{id:"thread:8x92m",title:"Why Rust is the future",author:"user:khadim",created:"2024-05-01"},{id:"thread:p09s1",title:"Local-first architecture",author:"user:sarah",created:"2024-05-02"},{id:"thread:7d66s",title:"Spooky vs ElectricSQL",author:"user:alex",created:"2024-05-03"}],m=()=>{const[t,i]=r.useState("events"),[a,n]=r.useState("0x8f2a9c"),o=l.find(s=>s.queryHash===a);return e.jsxs(e.Fragment,{children:[e.jsx("style",{children:d}),e.jsx("style",{children:y}),e.jsxs("div",{className:"browser-window",children:[e.jsxs("div",{className:"browser-header",children:[e.jsxs("div",{className:"window-controls",children:[e.jsx("div",{className:"control-dot red"}),e.jsx("div",{className:"control-dot yellow"}),e.jsx("div",{className:"control-dot green"})]}),e.jsxs("div",{className:"url-bar",children:[e.jsxs("svg",{width:"12",height:"12",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("rect",{x:"3",y:"11",width:"18",height:"11",rx:"2",ry:"2"}),e.jsx("path",{d:"M7 11V7a5 5 0 0 1 10 0v4"})]}),e.jsx("span",{children:"localhost:5173"})]})]}),e.jsxs("div",{className:"browser-body",children:[e.jsx("div",{className:"app-viewport",children:e.jsxs("div",{className:"app-placeholder",children:[e.jsxs("svg",{style:{opacity:.05,marginBottom:20},width:"100",height:"100",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"1",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"M12 2L2 7l10 5 10-5-10-5z"}),e.jsx("path",{d:"M2 17l10 5 10-5"}),e.jsx("path",{d:"M2 12l10 5 10-5"})]}),e.jsx("h1",{style:{fontSize:24,fontWeight:"bold",marginBottom:8},children:"My Spooky App"}),e.jsx("p",{style:{fontSize:14,opacity:.6},children:"Syncing active..."})]})}),e.jsx("div",{className:"devtools-container",children:e.jsxs("div",{className:"devtools-root",children:[e.jsxs("div",{className:"tabs",children:[e.jsx("div",{className:"toolbar-group",children:e.jsx("div",{className:"status-indicator",children:e.jsx("span",{className:"status-dot active"})})}),["events","queries","database","auth"].map(s=>e.jsx("button",{className:`tab-btn ${t===s?"active":""}`,onClick:()=>i(s),children:s.charAt(0).toUpperCase()+s.slice(1)},s)),e.jsxs("div",{className:"toolbar-group-right",children:[e.jsx("button",{className:"btn",children:"Refresh"}),e.jsx("button",{className:"btn",children:"Clear Events"})]})]}),e.jsxs("div",{className:"content",children:[e.jsx("div",{className:`tab-content ${t==="events"?"active":""}`,children:e.jsxs("div",{className:"events-container",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Events History"})}),e.jsx("div",{className:"events-list",children:p.map((s,c)=>e.jsxs("div",{className:"event-item",children:[e.jsxs("div",{className:"event-header",children:[e.jsx("span",{className:"event-type",children:s.type}),e.jsxs("span",{className:"event-time",children:[new Date(s.timestamp).toLocaleTimeString([],{hour12:!1}),".",String(s.timestamp%1e3).padStart(3,"0")]})]}),s.data&&e.jsx("div",{className:"event-payload",children:e.jsx("pre",{children:JSON.stringify(s.data,null,2)})})]},c))})]})}),e.jsx("div",{className:`tab-content ${t==="queries"?"active":""}`,children:e.jsxs("div",{className:"queries-container",children:[e.jsxs("div",{className:"queries-list",children:[e.jsx("div",{className:"queries-header",children:e.jsx("h2",{children:"Active Queries"})}),e.jsx("div",{className:"queries-list-content",children:l.map(s=>e.jsxs("div",{className:`query-item ${a===s.queryHash?"selected":""}`,onClick:()=>n(s.queryHash),children:[e.jsxs("div",{className:"query-header",children:[e.jsxs("span",{className:"query-hash",children:["#",s.queryHash]}),e.jsx("span",{className:`query-status status-${s.status}`,children:s.status})]}),e.jsxs("div",{className:"query-meta",children:["Updates: ",s.updateCount," | Size: ",s.dataSize,"B"]}),e.jsxs("div",{className:"query-preview",children:[s.query.substring(0,30),"..."]})]},s.queryHash))})]}),o?e.jsxs("div",{className:"query-detail",children:[e.jsxs("div",{className:"detail-header",children:[e.jsxs("h3",{children:["Query #",o.queryHash]}),e.jsx("span",{className:`query-status status-${o.status}`,children:o.status})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Created"}),e.jsx("div",{className:"detail-value",children:new Date(o.createdAt).toLocaleTimeString()})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Update Count"}),e.jsx("div",{className:"detail-value mono",children:o.updateCount})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Query"}),e.jsx("pre",{className:"query-code",children:o.query})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Variables"}),e.jsx("pre",{className:"query-code",children:JSON.stringify(o.variables,null,2)})]})]}):e.jsx("div",{className:"query-detail"})]})}),e.jsx("div",{className:`tab-content ${t==="database"?"active":""}`,children:e.jsxs("div",{className:"database-container",children:[e.jsxs("div",{className:"database-tables",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Tables"})}),e.jsx("div",{className:"tables-list",children:v.map(s=>e.jsxs("div",{className:`table-item ${s.name==="thread"?"selected":""}`,children:[e.jsx("span",{className:"table-name",children:s.name}),e.jsx("span",{className:"table-count",children:s.count})]},s.name))})]}),e.jsx("div",{className:"data-grid",children:e.jsxs("table",{children:[e.jsx("thead",{children:e.jsxs("tr",{children:[e.jsx("th",{children:"id"}),e.jsx("th",{children:"title"}),e.jsx("th",{children:"author"}),e.jsx("th",{children:"created"})]})}),e.jsx("tbody",{children:h.map(s=>e.jsxs("tr",{children:[e.jsx("td",{className:"text-primary",children:s.id}),e.jsxs("td",{className:"text-string",children:['"',s.title,'"']}),e.jsx("td",{className:"text-primary",children:s.author}),e.jsx("td",{className:"text-number",children:s.created})]},s.id))})]})})]})}),e.jsx("div",{className:`tab-content ${t==="auth"?"active":""}`,children:e.jsxs("div",{className:"events-container",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Authentication"})}),e.jsxs("div",{style:{padding:20,color:"#858585",fontFamily:"var(--sys-typescale-body-font)"},children:["User authenticated: ",e.jsx("span",{style:{color:"#4fc3f7"},children:"user:khadim"})]})]})})]})]})})]})]})]})};export{m as DevToolsSimulation};
