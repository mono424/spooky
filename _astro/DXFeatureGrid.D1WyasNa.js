import{j as e}from"./jsx-runtime.D_zvdyIk.js";import{r as d}from"./index.DPYYMTZ4.js";import"./_commonjsHelpers.CqkleIqs.js";const N=({figure:s,title:r,description:i,children:a,className:o=""})=>e.jsxs("div",{className:`bg-[#0a0a0a] overflow-hidden flex flex-col min-h-[560px] ${o}`,children:[e.jsx("div",{className:"px-6 pt-5 pb-2",children:e.jsx("span",{className:"font-mono text-[11px] text-text-muted tracking-wider uppercase",children:s})}),e.jsxs("div",{className:"flex-1 px-4 overflow-hidden relative",children:[e.jsx("div",{className:"h-full flex items-center justify-center",children:a}),e.jsx("div",{className:"absolute inset-x-0 bottom-0 h-12 pointer-events-none",style:{background:"linear-gradient(to top, #0a0a0a, transparent)"}}),e.jsx("div",{className:"absolute inset-x-0 top-0 h-32 pointer-events-none",style:{background:"linear-gradient(to bottom, #0a0a0a 10%, transparent)"}}),e.jsx("div",{className:"absolute inset-y-0 left-0 w-8 pointer-events-none",style:{background:"linear-gradient(to right, #0a0a0a, transparent)"}}),e.jsx("div",{className:"absolute inset-y-0 right-0 w-8 pointer-events-none",style:{background:"linear-gradient(to left, #0a0a0a, transparent)"}})]}),e.jsxs("div",{className:"px-6 pb-6 pt-4",children:[e.jsx("h3",{className:"text-lg font-semibold text-text-primary mb-1",children:r}),e.jsx("p",{className:"text-sm text-text-tertiary leading-relaxed",children:i})]})]}),C=`
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
`,F=`
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
`,R=[{type:"info",timestamp:1715011321152,data:{msg:"Spooky Sidecar initialized"}},{type:"info",timestamp:1715011321340,data:{msg:"Loading dbsp_worker.wasm... (2.1MB)"}},{type:"warn",timestamp:1715011322005,data:{msg:"State rehydration took 115ms",hints:["optimize-query"]}},{type:"info",timestamp:1715011325112,data:{msg:'Registered query "thread_list_view"'}}],S=[{queryHash:"0x8f2a9c",status:"active",createdAt:1715011325112,lastUpdate:1715011335112,updateCount:12,dataSize:1024,query:"SELECT * FROM thread ORDER BY created_at DESC",variables:{limit:10},data:[{id:"thread:1",title:"Why Rust?"}]},{queryHash:"0x99b1d4",status:"idle",createdAt:1715011326e3,lastUpdate:171501133e4,updateCount:5,dataSize:256,query:"SELECT count() FROM notifications",variables:{},data:{count:3}}],I=[{name:"user",count:12},{name:"thread",count:45},{name:"comment",count:182}],M=[{id:"thread:8x92m",title:"Why Rust is the future",author:"user:khadim",created:"2024-05-01"},{id:"thread:p09s1",title:"Local-first architecture",author:"user:sarah",created:"2024-05-02"},{id:"thread:7d66s",title:"Spooky vs ElectricSQL",author:"user:alex",created:"2024-05-03"}],B=()=>{const[s,r]=d.useState("events"),[i,a]=d.useState("0x8f2a9c"),[o,l]=d.useState(0),[h,y]=d.useState(0),c=["events","queries","database","auth"],n=S.find(t=>t.queryHash===i),u=t=>{l(t.targetTouches[0].clientX)},f=t=>{y(t.targetTouches[0].clientX)},b=()=>{if(!o||!h)return;const t=o-h,p=t>50,j=t<-50,x=c.indexOf(s);p&&x<c.length-1&&r(c[x+1]),j&&x>0&&r(c[x-1]),l(0),y(0)},[m,T]=d.useState(0),[g,k]=d.useState(!1),D=t=>{T(t.clientX),k(!0)},q=t=>{},L=t=>{if(!g)return;const p=m-t.clientX,j=p>80,x=p<-80,v=c.indexOf(s);j&&v<c.length-1&&r(c[v+1]),x&&v>0&&r(c[v-1]),k(!1)};return e.jsxs(e.Fragment,{children:[e.jsx("style",{children:C}),e.jsx("style",{children:F}),e.jsxs("div",{className:"browser-window",children:[e.jsxs("div",{className:"browser-header",children:[e.jsxs("div",{className:"window-controls",children:[e.jsx("div",{className:"control-dot red"}),e.jsx("div",{className:"control-dot yellow"}),e.jsx("div",{className:"control-dot green"})]}),e.jsxs("div",{className:"url-bar",children:[e.jsxs("svg",{width:"12",height:"12",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("rect",{x:"3",y:"11",width:"18",height:"11",rx:"2",ry:"2"}),e.jsx("path",{d:"M7 11V7a5 5 0 0 1 10 0v4"})]}),e.jsx("span",{children:"localhost:5173"})]})]}),e.jsxs("div",{className:"browser-body",children:[e.jsx("div",{className:"app-viewport",children:e.jsxs("div",{className:"app-placeholder",children:[e.jsxs("svg",{style:{opacity:.05,marginBottom:20},width:"100",height:"100",viewBox:"0 0 24 24",fill:"none",stroke:"currentColor",strokeWidth:"1",strokeLinecap:"round",strokeLinejoin:"round",children:[e.jsx("path",{d:"M12 2L2 7l10 5 10-5-10-5z"}),e.jsx("path",{d:"M2 17l10 5 10-5"}),e.jsx("path",{d:"M2 12l10 5 10-5"})]}),e.jsx("h1",{style:{fontSize:24,fontWeight:"bold",marginBottom:8},children:"My Spooky App"}),e.jsx("p",{style:{fontSize:14,opacity:.6},children:"Syncing active..."})]})}),e.jsx("div",{className:"devtools-container",children:e.jsxs("div",{className:"devtools-root",children:[e.jsxs("div",{className:"tabs",children:[e.jsx("div",{className:"toolbar-group",children:e.jsx("div",{className:"status-indicator",children:e.jsx("span",{className:"status-dot active"})})}),c.map(t=>e.jsx("button",{className:`tab-btn ${s===t?"active":""}`,onClick:()=>r(t),children:t.charAt(0).toUpperCase()+t.slice(1)},t)),e.jsxs("div",{className:"toolbar-group-right",children:[e.jsx("button",{className:"btn",children:"Refresh"}),e.jsx("button",{className:"btn",children:"Clear Events"})]})]}),e.jsxs("div",{className:"content",onTouchStart:u,onTouchMove:f,onTouchEnd:b,onPointerDown:D,onPointerMove:q,onPointerUp:L,style:{touchAction:"pan-y",cursor:g?"grabbing":"grab"},children:[e.jsx("div",{className:`tab-content ${s==="events"?"active":""}`,children:e.jsxs("div",{className:"events-container",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Events History"})}),e.jsx("div",{className:"events-list",children:R.map((t,p)=>e.jsxs("div",{className:"event-item",children:[e.jsxs("div",{className:"event-header",children:[e.jsx("span",{className:"event-type",children:t.type}),e.jsxs("span",{className:"event-time",children:[new Date(t.timestamp).toLocaleTimeString([],{hour12:!1}),".",String(t.timestamp%1e3).padStart(3,"0")]})]}),t.data&&e.jsx("div",{className:"event-payload",children:e.jsx("pre",{children:JSON.stringify(t.data,null,2)})})]},p))})]})}),e.jsx("div",{className:`tab-content ${s==="queries"?"active":""}`,children:e.jsxs("div",{className:"queries-container",children:[e.jsxs("div",{className:"queries-list",children:[e.jsx("div",{className:"queries-header",children:e.jsx("h2",{children:"Active Queries"})}),e.jsx("div",{className:"queries-list-content",children:S.map(t=>e.jsxs("div",{className:`query-item ${i===t.queryHash?"selected":""}`,onClick:()=>a(t.queryHash),children:[e.jsxs("div",{className:"query-header",children:[e.jsxs("span",{className:"query-hash",children:["#",t.queryHash]}),e.jsx("span",{className:`query-status status-${t.status}`,children:t.status})]}),e.jsxs("div",{className:"query-meta",children:["Updates: ",t.updateCount," | Size: ",t.dataSize,"B"]}),e.jsxs("div",{className:"query-preview",children:[t.query.substring(0,30),"..."]})]},t.queryHash))})]}),n?e.jsxs("div",{className:"query-detail",children:[e.jsxs("div",{className:"detail-header",children:[e.jsxs("h3",{children:["Query #",n.queryHash]}),e.jsx("span",{className:`query-status status-${n.status}`,children:n.status})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Created"}),e.jsx("div",{className:"detail-value",children:new Date(n.createdAt).toLocaleTimeString()})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Update Count"}),e.jsx("div",{className:"detail-value mono",children:n.updateCount})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Query"}),e.jsx("pre",{className:"query-code",children:n.query})]}),e.jsxs("div",{className:"detail-section",children:[e.jsx("div",{className:"detail-label",children:"Variables"}),e.jsx("pre",{className:"query-code",children:JSON.stringify(n.variables,null,2)})]})]}):e.jsx("div",{className:"query-detail"})]})}),e.jsx("div",{className:`tab-content ${s==="database"?"active":""}`,children:e.jsxs("div",{className:"database-container",children:[e.jsxs("div",{className:"database-tables",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Tables"})}),e.jsx("div",{className:"tables-list",children:I.map(t=>e.jsxs("div",{className:`table-item ${t.name==="thread"?"selected":""}`,children:[e.jsx("span",{className:"table-name",children:t.name}),e.jsx("span",{className:"table-count",children:t.count})]},t.name))})]}),e.jsx("div",{className:"data-grid",children:e.jsxs("table",{children:[e.jsx("thead",{children:e.jsxs("tr",{children:[e.jsx("th",{children:"id"}),e.jsx("th",{children:"title"}),e.jsx("th",{children:"author"}),e.jsx("th",{children:"created"})]})}),e.jsx("tbody",{children:M.map(t=>e.jsxs("tr",{children:[e.jsx("td",{className:"text-primary",children:t.id}),e.jsxs("td",{className:"text-string",children:['"',t.title,'"']}),e.jsx("td",{className:"text-primary",children:t.author}),e.jsx("td",{className:"text-number",children:t.created})]},t.id))})]})})]})}),e.jsx("div",{className:`tab-content ${s==="auth"?"active":""}`,children:e.jsxs("div",{className:"events-container",children:[e.jsx("div",{className:"events-header",children:e.jsx("h2",{children:"Authentication"})}),e.jsxs("div",{style:{padding:20,color:"#858585",fontFamily:"var(--sys-typescale-body-font)"},children:["User authenticated: ",e.jsx("span",{style:{color:"#4fc3f7"},children:"user:khadim"})]})]})})]})]})})]})]})]})},w=[{key:"schema",label:"Schema",sublabel:"schema.surql",accent:"#34d399",accentBorder:"rgba(52,211,153,0.5)"},{key:"types",label:"Types",sublabel:"schema.generated.ts",accent:"#38bdf8",accentBorder:"rgba(56,189,248,0.5)"},{key:"editor",label:"Editor",sublabel:"ThreadList.tsx",accent:"#a78bfa",accentBorder:"rgba(167,139,250,0.5)"}],A=[{tokens:[{text:"import",c:"#c084fc"},{text:" { useQuery } ",c:"#cbd5e1"},{text:"from",c:"#c084fc"},{text:" ",c:"#cbd5e1"},{text:'"spooky/solid"',c:"#fdba74"}]},{tokens:[{text:"import",c:"#c084fc"},{text:" { db } ",c:"#cbd5e1"},{text:"from",c:"#c084fc"},{text:" ",c:"#cbd5e1"},{text:'"./client"',c:"#fdba74"}]},{tokens:[{text:"",c:"#64748b"}]},{tokens:[{text:"export default function",c:"#c084fc"},{text:" ThreadList() {",c:"#cbd5e1"}]},{tokens:[{text:"  const",c:"#c084fc"},{text:" threads = ",c:"#cbd5e1"},{text:"useQuery",c:"#60a5fa"},{text:"(",c:"#64748b"}]},{tokens:[{text:"    db",c:"#60a5fa"},{text:".",c:"#64748b"},{text:"query",c:"#60a5fa"},{text:"(",c:"#64748b"},{text:'"thread"',c:"#fdba74"},{text:")",c:"#64748b"}]},{tokens:[{text:"      .",c:"#64748b"},{text:"related",c:"#60a5fa"},{text:"(",c:"#64748b"},{text:'"comments"',c:"#fdba74"},{text:")",c:"#64748b"}]},{tokens:[{text:"      .",c:"#64748b"},{text:"orderBy",c:"#60a5fa"},{text:"(",c:"#64748b"},{text:'"created_at"',c:"#fdba74"},{text:", ",c:"#64748b"},{text:'"desc"',c:"#fdba74"},{text:")",c:"#64748b"}]},{tokens:[{text:"      .",c:"#64748b"},{text:"build",c:"#60a5fa"},{text:"()",c:"#64748b"}]},{tokens:[{text:"  )",c:"#64748b"}]}],O=[{tokens:[{text:"// @generated by spooky-cli",c:"#546e7a"}]},{tokens:[{text:"export type",c:"#c084fc"},{text:" Thread = {",c:"#cbd5e1"}]},{tokens:[{text:"  id",c:"#cbd5e1"},{text:": ",c:"#64748b"},{text:"string",c:"#38bdf8"}]},{tokens:[{text:"  title",c:"#cbd5e1"},{text:": ",c:"#64748b"},{text:"string",c:"#38bdf8"}]},{tokens:[{text:"  body",c:"#cbd5e1"},{text:": ",c:"#64748b"},{text:"string",c:"#38bdf8"}]},{tokens:[{text:"  author",c:"#cbd5e1"},{text:": ",c:"#64748b"},{text:"Record",c:"#38bdf8"},{text:"<",c:"#64748b"},{text:"User",c:"#38bdf8"},{text:">",c:"#64748b"}]},{tokens:[{text:"  comments",c:"#cbd5e1"},{text:": ",c:"#64748b"},{text:"Comment",c:"#38bdf8"},{text:"[]",c:"#64748b"}]},{tokens:[{text:"  created_at",c:"#cbd5e1"},{text:": ",c:"#64748b"},{text:"Date",c:"#38bdf8"}]},{tokens:[{text:"}",c:"#cbd5e1"}]}],H=[{tokens:[{text:"DEFINE TABLE",c:"#c084fc"},{text:" thread ",c:"#cbd5e1"},{text:"SCHEMAFULL",c:"#c084fc"},{text:";",c:"#64748b"}]},{tokens:[{text:"DEFINE FIELD",c:"#c084fc"},{text:" title ",c:"#cbd5e1"},{text:"ON",c:"#c084fc"},{text:" thread ",c:"#cbd5e1"},{text:"TYPE",c:"#c084fc"},{text:" string",c:"#34d399"},{text:";",c:"#64748b"}]},{tokens:[{text:"DEFINE FIELD",c:"#c084fc"},{text:" body ",c:"#cbd5e1"},{text:"ON",c:"#c084fc"},{text:" thread ",c:"#cbd5e1"},{text:"TYPE",c:"#c084fc"},{text:" string",c:"#34d399"},{text:";",c:"#64748b"}]},{tokens:[{text:"DEFINE FIELD",c:"#c084fc"},{text:" author ",c:"#cbd5e1"},{text:"ON",c:"#c084fc"},{text:" thread ",c:"#cbd5e1"},{text:"TYPE",c:"#c084fc"},{text:" record",c:"#34d399"},{text:"<user>",c:"#34d399"},{text:";",c:"#64748b"}]},{tokens:[{text:"DEFINE FIELD",c:"#c084fc"},{text:" created_at ",c:"#cbd5e1"},{text:"ON",c:"#c084fc"},{text:" thread ",c:"#cbd5e1"},{text:"TYPE",c:"#c084fc"},{text:" datetime",c:"#34d399"},{text:";",c:"#64748b"}]},{tokens:[{text:"",c:"#64748b"}]},{tokens:[{text:"DEFINE TABLE",c:"#c084fc"},{text:" user ",c:"#cbd5e1"},{text:"SCHEMAFULL",c:"#c084fc"},{text:";",c:"#64748b"}]},{tokens:[{text:"DEFINE FIELD",c:"#c084fc"},{text:" username ",c:"#cbd5e1"},{text:"ON",c:"#c084fc"},{text:" user ",c:"#cbd5e1"},{text:"TYPE",c:"#c084fc"},{text:" string",c:"#34d399"},{text:";",c:"#64748b"}]},{tokens:[{text:"DEFINE FIELD",c:"#c084fc"},{text:" email ",c:"#cbd5e1"},{text:"ON",c:"#c084fc"},{text:" user ",c:"#cbd5e1"},{text:"TYPE",c:"#c084fc"},{text:" string",c:"#34d399"},{text:";",c:"#64748b"}]}],z={editor:A,types:O,schema:H},E=320,P=90;function _({layer:s,index:r,hovered:i,onHover:a}){const o=i===s.key,l=i!==null&&!o,h=o?16:0,y=r*P+h,c=z[s.key],n=o?`translateX(-50%) translateZ(${y}px) rotateZ(45deg) rotateX(-55deg) scale(1.15)`:`translateX(-50%) translateZ(${y}px)`;return e.jsx("div",{style:{position:"absolute",bottom:0,left:"50%",transform:n,opacity:l?0:1,filter:l?"brightness(0.5)":"brightness(1)",transition:"all 500ms cubic-bezier(0.4, 0, 0.2, 1)",pointerEvents:"none",zIndex:o?10:r},children:e.jsxs("div",{style:{width:E,background:"#111113",border:`1px solid ${o?s.accentBorder:"rgba(255,255,255,0.08)"}`,borderRadius:10,transition:"all 500ms cubic-bezier(0.4, 0, 0.2, 1)",display:"flex",flexDirection:"column",boxShadow:o?`0 0 40px ${s.accent}15, 0 8px 30px rgba(0,0,0,0.5)`:"0 4px 16px rgba(0,0,0,0.4)"},children:[e.jsxs("div",{style:{display:"flex",alignItems:"center",gap:8,padding:"8px 12px",background:"#0a0a0c",borderBottom:`1px solid ${o?s.accentBorder:"rgba(255,255,255,0.06)"}`,borderRadius:"10px 10px 0 0",transition:"all 300ms ease"},children:[e.jsxs("div",{style:{display:"flex",gap:4},children:[e.jsx("div",{style:{width:6,height:6,borderRadius:"50%",background:"rgba(255,255,255,0.08)"}}),e.jsx("div",{style:{width:6,height:6,borderRadius:"50%",background:"rgba(255,255,255,0.08)"}}),e.jsx("div",{style:{width:6,height:6,borderRadius:"50%",background:"rgba(255,255,255,0.08)"}})]}),e.jsx("span",{style:{fontFamily:"'JetBrains Mono', monospace",fontSize:9,fontWeight:500,color:o?s.accent:"rgba(255,255,255,0.3)",letterSpacing:"0.02em",transition:"color 300ms ease"},children:s.sublabel})]}),e.jsx("div",{style:{padding:"10px 16px 12px"},children:c.map((u,f)=>e.jsx("div",{style:{fontFamily:"'JetBrains Mono', monospace",fontSize:10,lineHeight:"17px",whiteSpace:"nowrap"},children:u.tokens.map((b,m)=>e.jsx("span",{style:{color:b.c},children:b.text},m))},f))})]})})}function U({layer:s}){const r=z[s.key];return e.jsxs("div",{style:{background:"#111113",border:"1px solid rgba(255,255,255,0.08)",borderRadius:10},children:[e.jsxs("div",{style:{display:"flex",alignItems:"center",gap:8,padding:"10px 14px",background:"#0a0a0c",borderBottom:"1px solid rgba(255,255,255,0.06)",borderRadius:"10px 10px 0 0"},children:[e.jsxs("div",{style:{display:"flex",gap:4},children:[e.jsx("div",{style:{width:6,height:6,borderRadius:"50%",background:"rgba(255,255,255,0.08)"}}),e.jsx("div",{style:{width:6,height:6,borderRadius:"50%",background:"rgba(255,255,255,0.08)"}}),e.jsx("div",{style:{width:6,height:6,borderRadius:"50%",background:"rgba(255,255,255,0.08)"}})]}),e.jsx("span",{style:{fontFamily:"'JetBrains Mono', monospace",fontSize:10,fontWeight:500,color:s.accent},children:s.sublabel})]}),e.jsx("div",{style:{padding:"8px 14px 10px"},children:r.map((i,a)=>e.jsx("div",{style:{fontFamily:"'JetBrains Mono', monospace",fontSize:11,lineHeight:"18px",whiteSpace:"nowrap"},children:i.tokens.map((o,l)=>e.jsx("span",{style:{color:o.c},children:o.text},l))},a))})]})}const $=()=>{const[s,r]=d.useState(null),i=480;return e.jsxs("div",{style:{width:"100%"},children:[e.jsxs("div",{className:"hidden md:flex",style:{justifyContent:"center",alignItems:"center",position:"relative",minHeight:i},children:[e.jsx("div",{style:{position:"relative",width:E+80,height:i,perspective:1e3,marginLeft:-60},children:e.jsx("div",{style:{position:"relative",width:"100%",height:"100%",transformStyle:"preserve-3d",transform:"rotateX(55deg) rotateZ(-45deg)"},children:w.map((a,o)=>e.jsx(_,{layer:a,index:o,hovered:s,onHover:r},a.key))})}),e.jsx("div",{style:{position:"absolute",left:12,top:0,bottom:0,display:"flex",flexDirection:"column",justifyContent:"space-between",paddingTop:30,paddingBottom:90,pointerEvents:"auto"},children:[...w].reverse().map(a=>{const o=s===a.key;return e.jsxs("div",{onMouseEnter:()=>r(a.key),onMouseLeave:()=>r(null),style:{display:"flex",alignItems:"center",gap:10,opacity:s===null||o?1:0,transition:"all 300ms cubic-bezier(0.4, 0, 0.2, 1)",cursor:"pointer"},children:[e.jsx("span",{style:{fontSize:10,fontWeight:600,color:o?a.accent:"rgba(255,255,255,0.5)",letterSpacing:"0.08em",textTransform:"uppercase",fontFamily:"'JetBrains Mono', monospace",transition:"color 300ms ease",whiteSpace:"nowrap"},children:a.label}),e.jsx("div",{style:{width:32,height:1,background:o?`linear-gradient(90deg, ${a.accent}, transparent)`:"rgba(255,255,255,0.15)",transition:"background 300ms ease"}})]},a.key)})})]}),e.jsx("div",{className:"flex md:hidden",style:{flexDirection:"column",gap:8,padding:"0 4px"},children:[...w].reverse().map(a=>e.jsx(U,{layer:a},a.key))})]})},W=()=>e.jsx("div",{className:"rounded-2xl border border-surface-border overflow-hidden bg-surface-border/50",children:e.jsxs("div",{className:"grid grid-cols-1 md:grid-cols-2 gap-px",children:[e.jsx(N,{figure:"FIG. 3.1",title:"Schema-First Intelligence",description:"Define your schema once — get type-safe clients, context-aware autocomplete, and real-time validation across your entire stack.",className:"md:rounded-l-2xl",children:e.jsxs("div",{className:"w-full h-full flex items-center justify-center pt-8 pb-4 relative",style:{paddingRight:60},children:[e.jsx($,{}),e.jsx("div",{className:"absolute inset-x-0 bottom-0 h-28 pointer-events-none z-10",style:{background:"linear-gradient(to top, #0a0a0a 15%, transparent)"}}),e.jsx("div",{className:"absolute inset-y-0 right-0 w-24 pointer-events-none z-10",style:{background:"linear-gradient(to left, #0a0a0a 10%, transparent)"}})]})}),e.jsx(N,{figure:"FIG. 3.2",title:"Effortlessly Transparent",description:"A complete suite of developer tools embedded directly in your browser. Inspect state, monitor queries, and track events.",className:"md:rounded-r-2xl",children:e.jsx("div",{className:"w-full relative",style:{perspective:800,perspectiveOrigin:"50% 100%"},children:e.jsx("div",{style:{transform:"rotateX(8deg) scale(0.82) translateY(-60px)",transformOrigin:"bottom center"},children:e.jsx(B,{})})})})]})});export{W as DXFeatureGrid};
