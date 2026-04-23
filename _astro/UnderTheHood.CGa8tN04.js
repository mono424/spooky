import{j as t}from"./jsx-runtime.u17CrQMm.js";import{r as p}from"./index.CIDUnBje.js";import{r as U}from"./index.BNM0o4Jr.js";var f=function(){return f=Object.assign||function(a){for(var i,r=1,s=arguments.length;r<s;r++){i=arguments[r];for(var o in i)Object.prototype.hasOwnProperty.call(i,o)&&(a[o]=i[o])}return a},f.apply(this,arguments)};function g(e,a){var i={};for(var r in e)Object.prototype.hasOwnProperty.call(e,r)&&a.indexOf(r)<0&&(i[r]=e[r]);if(e!=null&&typeof Object.getOwnPropertySymbols=="function")for(var s=0,r=Object.getOwnPropertySymbols(e);s<r.length;s++)a.indexOf(r[s])<0&&Object.prototype.propertyIsEnumerable.call(e,r[s])&&(i[r[s]]=e[r[s]]);return i}var M=p.createContext(null),z=function(){var e=p.useContext(M);if(!e)throw new Error("Hologram components must be used within HologramSticker.Root");return e},H=function(e){var a=e.children,i=p.useState(!0),r=i[0];i[1];var s=p.useState(!1),o=s[0],l=s[1],c=p.useState({x:"0",y:"0"}),d=c[0],n=c[1],x=p.useState(!0),u=x[0];x[1];var h=p.useRef(null),m=p.useRef(null),b=p.useRef(null),k=p.useRef(null);p.useEffect(function(){var C=function(E){if(!(!r&&!o)){var S=o?m.current:h.current,v=S?.getBoundingClientRect();if(v){var q=E.clientX-v.x,F=E.clientY-v.y,O=q/v.width-.5,T=F/v.height-.5,G=Math.max(-1,Math.min(1,O*2)),L=Math.max(-1,Math.min(1,T*2));n({x:G.toFixed(2),y:L.toFixed(2)}),k.current&&(k.current.style.setProperty("--sticker-pointer-x",G.toString()),k.current.style.setProperty("--sticker-pointer-y",L.toString()))}}},w=o?m.current:h.current;return w&&w.addEventListener("pointermove",C),function(){w&&w.removeEventListener("pointermove",C)}},[r,o]),p.useEffect(function(){k.current&&(k.current.dataset.explode=o.toString())},[o]);var N={isActive:r,isExploded:o,setIsExploded:l,pointerPos:d,showGlare:u,cardRef:h,minimapRef:m,sceneRef:b,rootRef:k};return t.jsx(M.Provider,{value:N,children:a})},D=function(e){var a=e.children,i=e.className,r=i===void 0?"":i,s=e.style,o=g(e,["children","className","style"]),l=z().rootRef;return t.jsx("div",f({ref:l,className:"sticker-root ".concat(r),style:s},o,{children:a}))},Z=function(e){var a=e.children,i=e.className,r=i===void 0?"":i,s=e.style,o=g(e,["children","className","style"]);return t.jsx(H,{children:t.jsx(D,f({className:r,style:s},o,{children:a}))})},P=p.forwardRef(function(e,a){var i=e.children,r=e.className,s=r===void 0?"":r,o=g(e,["children","className"]),l=z().sceneRef;return t.jsx("div",f({ref:function(c){a&&(typeof a=="function"?a(c):a.current=c),l&&"current"in l&&(l.current=c)},className:"sticker-scene ".concat(s)},o,{children:i}))});P.displayName="Scene";var A=p.forwardRef(function(e,a){var i=e.children,r=e.className,s=r===void 0?"":r,o=e.width,l=e.aspectRatio,c=g(e,["children","className","width","aspectRatio"]),d=z(),n=d.isActive,x=d.isExploded,u=d.cardRef,h=f(f({},o&&{width:"".concat(o,"px")}),l&&{aspectRatio:l.toString()});return t.jsx("article",f({ref:function(m){a&&(typeof a=="function"?a(m):a.current=m),u&&"current"in u&&(u.current=m)},className:"sticker-card ".concat(s," ").concat(n?"active":""," ").concat(x?"exploded":""),style:h,"data-active":n},c,{children:t.jsx("div",{className:"sticker-content",children:i})}))});A.displayName="Card";var W=function(e){var a=e.src,i=e.alt,r=i===void 0?"":i,s=e.className,o=s===void 0?"":s,l=e.opacity,c=e.objectFit,d=e.scale,n=e.parallax,x=n===void 0?!1:n,u=e.style,h=g(e,["src","alt","className","opacity","objectFit","scale","parallax","style"]),m=f(f({},u),l!==void 0&&{opacity:l}),b=f(f({},c&&{objectFit:c}),d!==void 0&&{scale:d});return t.jsx("div",f({className:"sticker-img-layer ".concat(x?"sticker-img-layer--parallax":"sticker-img-layer--static"," ").concat(o),style:m},h,{children:t.jsx("img",{src:a,alt:r,style:b})}))},X=function(e){var a=e.children,i=e.className,r=i===void 0?"":i,s=e.maskUrl,o=e.maskSize,l=o===void 0?"contain":o,c=e.textureUrl,d=c===void 0?"https://assets.codepen.io/605876/figma-texture.png":c,n=e.opacity,x=n===void 0?.4:n;e.mode;var u=e.mixBlendMode,h=u===void 0?"multiply":u,m=e.textureSize,b=m===void 0?"4cqi":m,k=g(e,["children","className","maskUrl","maskSize","textureUrl","opacity","mode","mixBlendMode","textureSize"]),N=f({"--pattern-opacity":x,"--pattern-mix-blend-mode":h,"--pattern-texture-size":b,"--pattern-url":"url(".concat(d,")")},s&&{"--pattern-mask-url":"url(".concat(s,")"),"--pattern-mask-size":l});return t.jsx("div",f({className:"sticker-pattern ".concat(s?"sticker-pattern--mask":""," ").concat(r),style:N},k,{children:a}))},Y=function(e){var a=e.children,i=e.className,r=i===void 0?"":i,s=e.imageUrl,o=s===void 0?"https://assets.codepen.io/605876/shopify-pattern.svg":s,l=e.opacity,c=l===void 0?1:l,d=g(e,["children","className","imageUrl","opacity"]),n={"--watermark-url":"url(".concat(o,")"),"--watermark-opacity":c};return t.jsx("div",f({className:"sticker-watermark ".concat(r),style:n},d,{children:a}))},_=function(e){var a=e.className,i=a===void 0?"":a,r=e.intensity,s=r===void 0?1:r,o=e.variant,l=o===void 0?"default":o;e.colors;var c=g(e,["className","intensity","variant","colors"]),d=l==="debug"?"sticker-refraction--debug":"sticker-refraction";return t.jsxs(t.Fragment,{children:[t.jsx("div",f({className:"".concat(d," sticker-refraction-1 ").concat(i),style:{"--intensity":s}},c)),t.jsx("div",f({className:"".concat(d," sticker-refraction-2 ").concat(i),style:{"--intensity":s}},c))]})},$=function(e){var a=e.children,i=e.className,r=i===void 0?"":i,s=g(e,["children","className"]);return t.jsx("div",f({className:"sticker-content ".concat(r)},s,{children:a}))},J=function(e){var a=e.className,i=a===void 0?"":a,r=e.intensity,s=r===void 0?1:r,o=g(e,["className","intensity"]);return t.jsx("div",f({className:"sticker-spotlight ".concat(i),style:{"--spotlight-intensity":s}},o))},K=function(e){var a=e.className,i=a===void 0?"":a,r=g(e,["className"]);return t.jsx("div",f({className:"sticker-glare-container ".concat(i)},r,{children:t.jsx("div",{className:"sticker-glare animate"})}))};function Q(e,a){a===void 0&&(a={});var i=a.insertAt;if(!(typeof document>"u")){var r=document.head||document.getElementsByTagName("head")[0],s=document.createElement("style");s.type="text/css",i==="top"&&r.firstChild?r.insertBefore(s,r.firstChild):r.appendChild(s),s.styleSheet?s.styleSheet.cssText=e:s.appendChild(document.createTextNode(e))}}var V='.sticker-root{opacity:1;visibility:visible}.sticker-card{max-height:calc(var(--sticker-card-width, 260px)*7/5);max-width:var(--sticker-card-width,260px)}.sticker-background img,.sticker-frame img,.sticker-img-layer img{display:block;height:auto;max-width:100%}:root{--sticker-card-width:260px;--sticker-card-border-radius:8cqi;--sticker-pointer-x:0;--sticker-pointer-y:0;--sticker-parallax-img-x:5%;--sticker-parallax-img-y:5%;--sticker-rotate-x:25deg;--sticker-rotate-y:-20deg;--sticker-border-color:#404040}*,:after,:before{box-sizing:border-box}.sr-only{clip:rect(0,0,0,0);border-width:0;height:1px;margin:-1px;overflow:hidden;padding:0;position:absolute;white-space:nowrap;width:1px}.sticker-root{align-items:center;display:flex;justify-content:center;min-height:400px;position:relative;width:100%}.sticker-scene{perspective:1000px;position:relative;transform:translateZ(100vmin)}.sticker-arrow{color:#fff;display:inline-block;font-family:Gloria Hallelujah,Comic Sans MS,cursive,sans-serif;font-size:.875rem;left:50%;opacity:0;position:absolute;rotate:10deg;top:50%;transition:opacity .26s ease-out;translate:calc(-40% + var(--sticker-card-width)*-1) 0;width:80px;z-index:99999}.sticker-arrow.visible{opacity:.8}.sticker-arrow span{display:inline-block;rotate:-24deg;translate:30% 100%}.sticker-arrow svg{left:0;rotate:10deg;rotate:-25deg;scale:1 -1;translate:120% 20%;width:80%}[data-explode=true] .sticker-arrow{opacity:0}@media (max-width:580px){.sticker-arrow{translate:-50% calc(var(--sticker-card-width)*7/5*.5)}.sticker-arrow span{translate:80% 160%}.sticker-arrow svg{bottom:100%;rotate:190deg;top:unset;translate:0 0}}.sticker-minimap{aspect-ratio:5/7;background:grey;border:4px solid var(--sticker-border-color);border-radius:6px;cursor:pointer;left:50%;pointer-events:none;position:fixed;top:50%;transform:translateZ(100vmin);transition:all .3s;translate:calc(var(--sticker-card-width)*1) -50%;visibility:hidden;width:60px;z-index:999999}.sticker-minimap.visible{pointer-events:all;visibility:visible}.sticker-minimap:after{content:"trackpad";font-family:Sora,system-ui,-apple-system,Segoe UI,Roboto,sans-serif;font-size:.875rem;left:100%;opacity:.35;pointer-events:none;top:50%;transform:translate(-50%,-50%) rotate(-90deg) translateY(100%)}.sticker-minimap:after,.sticker-minimap__stats{color:#fff;mix-blend-mode:difference;position:absolute}.sticker-minimap__stats{display:flex;flex-direction:column;font-family:monospace;font-size:.75rem;left:0;opacity:.7;right:0;top:calc(100% + .5rem)}.sticker-minimap__stats span{white-space:nowrap}.sticker-card{aspect-ratio:5/7;background:transparent;color:#1a1a1a;container-type:inline-size;display:block;font-family:Sora,system-ui,-apple-system,Segoe UI,Roboto,sans-serif;overflow:hidden;perspective:1600px;position:relative;transform-style:preserve-3d;width:var(--sticker-card-width)}.sticker-card :after,.sticker-card :before,.sticker-card img{will-change:translate,scale,filter}.sticker-card.active{transition:transform .2s}[data-explode=false] .sticker-card.active:hover{animation:set .2s backwards;transform:rotateX(calc(var(--sticker-pointer-y)*var(--sticker-rotate-x))) rotateY(calc(var(--sticker-pointer-x)*var(--sticker-rotate-y)));transition:transform 0s}@keyframes set{0%{transform:rotateX(0deg) rotateY(0deg)}}.sticker-card:not(:hover) img{transition:translate .2s}.sticker-card.exploded{pointer-events:none;transform:rotateX(-24deg) rotateY(32deg) rotateX(90deg);transition:transform .2s .2s}[data-explode=true] .sticker-card .sticker-pattern,[data-explode=true] .sticker-card .sticker-spotlight,[data-explode=true] .sticker-card .sticker-watermark{mix-blend-mode:unset}.sticker-flip-button{-webkit-tap-highlight-color:transparent;background:none;border:none;cursor:pointer;inset:0;opacity:0;position:absolute;z-index:100}.sticker-card *{pointer-events:none}.sticker-flip-button{pointer-events:all}.sticker-content{border-radius:var(--sticker-card-border-radius);inset:0;position:absolute;transform-style:preserve-3d;transition:rotate .26s ease-out}.sticker-content.flipped{rotate:180deg y}.sticker-content>:not(.sticker-debug:not(.sticker-debug--clipped)){clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius))}.sticker-emboss{filter:url(#sticker-lighting);position:relative}.sticker-emboss:before{bottom:0;color:#fff;content:"TechTrades © 2025";display:flex;font-size:1.5cqi;height:calc(var(--sticker-card-border-radius)*.5);left:50%;mix-blend-mode:difference;opacity:.8;place-items:center;position:absolute;translate:-50% 0;z-index:100}.sticker-emboss:after{border:calc(var(--sticker-card-border-radius)*.5 + 1px) solid var(--sticker-border-color);content:"";inset:-1px;z-index:99}.sticker-background,.sticker-emboss:after{border-radius:var(--sticker-card-border-radius);position:absolute}.sticker-background{inset:0}.sticker-background img{border-radius:var(--sticker-card-border-radius);height:100%;object-fit:cover;width:100%}.sticker-img-layer{border-radius:var(--sticker-card-border-radius);clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius));inset:0;opacity:1;position:absolute}.sticker-img-layer:before{background:transparent;content:"";inset:0;position:absolute}.sticker-img-layer img{filter:brightness(.85);height:100%;inset:0;object-fit:cover;position:absolute;scale:1;transition:translate .2s;width:100%}[data-explode=false] .sticker-card.active:hover .sticker-img-layer--parallax img,[data-explode=true]:has(.sticker-minimap:hover) .sticker-img-layer--parallax img{animation:set-img .2s backwards;transition:transform 0s;translate:calc(var(--sticker-pointer-x)*var(--sticker-parallax-img-x)) calc(var(--sticker-pointer-y)*var(--sticker-parallax-img-y))}[data-explode=false] .sticker-card.active:hover .sticker-frame img,[data-explode=true]:has(.sticker-minimap:hover) .sticker-frame img{animation:set-img .2s backwards;transition:transform 0s;translate:calc(var(--sticker-pointer-x)*var(--sticker-parallax-img-x)) calc(var(--sticker-pointer-y)*var(--sticker-parallax-img-y))}@keyframes set-img{0%{translate:0 0}}.sticker-debug{border-radius:var(--sticker-card-border-radius);inset:0;opacity:0;position:absolute;visibility:hidden}.sticker-debug[data-visible=true]{visibility:visible}.sticker-debug:after{border:4px dashed;border-radius:var(--sticker-card-border-radius);content:"";inset:0;position:absolute}.sticker-debug--clipped{clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius))}.sticker-debug .sticker-refraction--debug{opacity:.2}.sticker-debug--clipped .sticker-refraction--debug{opacity:1}[data-explode=true] .sticker-debug{visibility:visible}[data-explode=true] .sticker-debug:not(.sticker-debug--clipped) .sticker-refraction--debug{opacity:.2}.sticker-pattern{border-radius:var(--sticker-card-border-radius);clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius));filter:saturate(.8) contrast(1) brightness(1);inset:0;mask:var(--pattern-url,url(https://assets.codepen.io/605876/figma-texture.png)) 50% 50% /var(--pattern-texture-size,4cqi) var(--pattern-texture-size,4cqi);mix-blend-mode:var(--pattern-mix-blend-mode,multiply);opacity:var(--pattern-opacity,.4);position:absolute}.sticker-pattern:before{background:#ccc;content:"";inset:0;position:absolute}.sticker-pattern--mask{mask:var(--pattern-mask-url) center /var(--pattern-mask-size,contain) no-repeat,var(--pattern-url) 50% 50% /var(--pattern-texture-size,4cqi) var(--pattern-texture-size,4cqi);-webkit-mask:var(--pattern-mask-url) center /var(--pattern-mask-size,contain) no-repeat,var(--pattern-url) 50% 50% /var(--pattern-texture-size,4cqi) var(--pattern-texture-size,4cqi);mask-composite:intersect;-webkit-mask-composite:source-in}.sticker-watermark{border-radius:var(--sticker-card-border-radius);clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius));filter:saturate(.9) contrast(1.1) brightness(1.2);inset:0;mask:var(--watermark-url,url(https://assets.codepen.io/605876/shopify-pattern.svg)) 50% 50% /14cqi 14cqi repeat;mix-blend-mode:hard-light;opacity:var(--watermark-opacity,1);position:absolute}.sticker-watermark:before{background:hsla(0,0%,100%,.2);content:"";inset:0;position:absolute}.sticker-refraction,.sticker-spotlight:before{opacity:0;transition:opacity .2s ease-out}.sticker-card.active:hover .sticker-spotlight:before,.sticker-card.active:hover :not(.sticker-debug) .sticker-refraction,[data-explode=true]:has(.sticker-minimap:hover) .sticker-refraction,[data-explode=true]:has(.sticker-minimap:hover) .sticker-spotlight:before{opacity:1}[data-explode=true]:has(.sticker-minimap:hover) .sticker-debug:not(.sticker-debug--clipped) .sticker-refraction{opacity:.2}.sticker-refraction{aspect-ratio:1/1;filter:saturate(calc(var(--intensity, 1)*2));position:absolute;width:500%;will-change:translate,scale,filter}.sticker-refraction-1{background:radial-gradient(circle at 0 100%,transparent 10%,#ffa299,#3f9,#6e9cf7,transparent 60%);bottom:0;left:0;scale:min(1,calc(.15 + var(--sticker-pointer-x)*.25));transform-origin:0 100%;translate:clamp(-10%,calc(-10% + var(--sticker-pointer-x)*10%),10%) calc(max(0%, var(--sticker-pointer-y) * -1 * 10%))}.sticker-refraction-2{background:radial-gradient(circle at 100% 0,transparent 10%,#ffa299,#3f9,#6e9cf7,transparent 60%);right:0;scale:min(1,calc(.15 + var(--sticker-pointer-x)*-.65));top:0;transform-origin:100% 0;translate:clamp(-10%,calc(10% - var(--sticker-pointer-x)*-10%),10%) calc(min(0%, var(--sticker-pointer-y) * -10%))}.sticker-frame{border-radius:var(--sticker-card-border-radius);inset:0;opacity:1;position:absolute;z-index:2}.sticker-frame img{filter:saturate(.8) contrast(1.2) drop-shadow(0 0 10cqi hsl(0 0% 10%/.75));height:100%;inset:0;object-fit:cover;position:absolute;scale:1.1;width:100%}.sticker-spotlight{clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius));inset:0;mix-blend-mode:overlay;position:absolute;z-index:10}.sticker-spotlight:after{border:4px dashed;border-radius:var(--sticker-card-border-radius);content:"";inset:0;opacity:0;position:absolute}[data-explode=true] .sticker-spotlight:after{opacity:1}.sticker-spotlight:before{aspect-ratio:1;background:radial-gradient(hsl(0 0% 100%/calc(var(--spotlight-intensity, 1)*.4)) 0 2%,hsl(0 0% 10%/calc(var(--spotlight-intensity, 1)*.2)) 20%);content:"";filter:brightness(1.2) contrast(1.2);left:50%;opacity:0;position:absolute;top:50%;transition:opacity .2s ease-out;translate:calc(-50% + var(--sticker-pointer-x)*20%) calc(-50% + var(--sticker-pointer-y)*20%);width:500%}.sticker-glare-container{border-radius:var(--sticker-card-border-radius);clip-path:inset(0 0 0 0 round var(--sticker-card-border-radius));inset:0;overflow:hidden;position:absolute}.sticker-glare{background:linear-gradient(-65deg,transparent 0 40%,#fff 40% 50%,transparent 30% 50%,transparent 50% 55%,#fff 55% 60%,transparent 60% 100%);inset:0;opacity:.5;position:absolute;transform:translateX(100%)}.sticker-glare.animate{animation:glareSwipe .65s ease-in-out .75s forwards}@keyframes glareSwipe{to{transform:translateX(-100%)}}.sticker-wordmark{height:max-content;left:50%;position:absolute;translate:-50% 0;width:70%}.sticker-wordmark--top{top:12%}.sticker-wordmark--bottom{bottom:12%;rotate:180deg}.sticker-wordmark:after{color:#fff;content:"™";mix-blend-mode:difference;position:absolute;right:0;top:100%}.sticker-wordmark img{height:auto;position:static;width:100%}.sticker-gemstone{filter:hue-rotate(320deg);height:auto;left:50%;position:absolute;top:50%;translate:-50% -50%;width:50%}.sticker-controls{display:flex;gap:.5rem;position:fixed;right:1rem;top:1rem;transform:translateZ(200vmin);z-index:999999}.sticker-controls button{color:#fff;mix-blend-mode:difference}.sticker-overlay{inset:0;opacity:var(--overlay-opacity,1);position:absolute;z-index:1}.sticker-overlay img{height:100%;object-fit:cover;width:100%}[data-explode=true] .sticker-content,[data-explode=true] .sticker-debug,[data-explode=true] .sticker-frame,[data-explode=true] .sticker-glare-container,[data-explode=true] .sticker-img-layer,[data-explode=true] .sticker-pattern,[data-explode=true] .sticker-spotlight,[data-explode=true] .sticker-spotlight:after,[data-explode=true] .sticker-watermark{transition-delay:.4s;transition-duration:.2s;transition-property:transform,opacity}[data-explode=true] .sticker-img-layer{transform:translateZ(-240px)}[data-explode=true] .sticker-debug:not(.sticker-debug--clipped){opacity:.3;transform:translateZ(-160px)}[data-explode=true] .sticker-debug--clipped{opacity:.5;transform:translateZ(-120px)}[data-explode=true] .sticker-pattern{transform:translateZ(-80px)}[data-explode=true] .sticker-watermark{transform:translateZ(-40px)}[data-explode=true] .sticker-content{transform:translateZ(40px)}[data-explode=true] .sticker-frame{transform:translateZ(80px)}[data-explode=true] .sticker-spotlight{transform:translateZ(160px)}[data-explode=true] .sticker-glare-container{transform:translateZ(240px)}';Q(V);var y={Root:Z,Scene:P,Card:A,ImageLayer:W,Pattern:X,Watermark:Y,Refraction:_,Content:$,Spotlight:J,Glare:K};function ee(){const e=p.useRef(null),[a,i]=p.useState(0),r=p.useRef(!1),s=p.useRef(null),o=p.useCallback(()=>{const c=e.current;if(!c)return;const d=c.getBoundingClientRect(),n=window.innerHeight,x=n*.6,u=n*.25,h=(x-d.top)/(x-u);i(Math.min(1,Math.max(0,h)))},[]),l=p.useCallback(()=>{r.current&&s.current===null&&(s.current=requestAnimationFrame(()=>{s.current=null,o()}))},[o]);return p.useEffect(()=>{const c=e.current;if(!c)return;const d=new IntersectionObserver(([n])=>{r.current=n.isIntersecting,n.isIntersecting&&o()},{rootMargin:"0px 0px 0px 0px",threshold:0});return d.observe(c),window.addEventListener("scroll",l,{passive:!0}),()=>{d.disconnect(),window.removeEventListener("scroll",l),s.current!==null&&cancelAnimationFrame(s.current)}},[l,o]),{ref:e,progress:a}}function B({segments:e,trailing:a,className:i}){const{ref:r,progress:s}=ee(),o=[];for(const n of e){const x=n.text.split(/\s+/).filter(Boolean);for(const u of x)o.push({word:u,preRevealed:!!n.preRevealed})}const l=o.filter(n=>!n.preRevealed),c=Math.round(s*l.length);let d=0;return t.jsxs("p",{ref:r,className:i,children:[o.map((n,x)=>{if(n.preRevealed)return t.jsxs("span",{children:[t.jsx("span",{className:"text-white",children:n.word})," "]},x);const u=d<c;return d++,t.jsxs("span",{children:[t.jsx("span",{className:u?"text-white":"text-gray-500",style:{transition:"color 0.15s ease"},children:n.word})," "]},x)}),a]})}const R=.33,I=.66,j=.3,te=e=>{p.useEffect(()=>{const a=e.current;if(!a)return;let i=null,r=0;const s=()=>{if(r=0,i||(i=a.querySelector(".sticker-card")),!i)return;const l=document.documentElement,c=l.scrollHeight-window.innerHeight,d=c>0?l.scrollTop/c:0;let n;if(d<=R)n=-j;else if(d>=I)n=j;else{const x=(d-R)/(I-R);n=-j+2*j*x}i.style.setProperty("--sticker-pointer-x",(-n*.9).toFixed(3)),i.style.setProperty("--sticker-pointer-y",n.toFixed(3))},o=()=>{r||(r=requestAnimationFrame(s))};return r=requestAnimationFrame(s),window.addEventListener("scroll",o,{passive:!0}),window.addEventListener("resize",o,{passive:!0}),()=>{window.removeEventListener("scroll",o),window.removeEventListener("resize",o),r&&cancelAnimationFrame(r)}},[e])},re=(e,a,i="#f8fafc",r=300,s=12)=>`data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 700" style="color:${i}"><defs><filter id="iconGlow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur in="SourceAlpha" stdDeviation="8"/><feOffset dx="0" dy="0" result="offsetblur"/><feFlood flood-color="#000" flood-opacity="0.45"/><feComposite in2="offsetblur" operator="in"/><feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>`+e+(a?`<g transform="translate(250 ${r}) scale(${s}) translate(-12 -12)" filter="url(#iconGlow)">${a}</g>`:"")+"</svg>")}`,ie=`
<defs>
  <linearGradient id="r-sky" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#1a0436"/>
    <stop offset="0.4" stop-color="#4a0e5e"/>
    <stop offset="0.65" stop-color="#ff3d6b"/>
    <stop offset="0.82" stop-color="#ff8a3d"/>
    <stop offset="1" stop-color="#ffcf6b"/>
  </linearGradient>
  <radialGradient id="r-halo" cx="50%" cy="40%" r="55%">
    <stop offset="0" stop-color="#ffb56b" stop-opacity="0.55"/>
    <stop offset="1" stop-color="#ffb56b" stop-opacity="0"/>
  </radialGradient>
  <linearGradient id="r-sun" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#fff38a"/>
    <stop offset="0.55" stop-color="#ff7a3a"/>
    <stop offset="1" stop-color="#e0245e"/>
  </linearGradient>
  <linearGradient id="r-floor" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#3a0f5a"/>
    <stop offset="1" stop-color="#0f0420"/>
  </linearGradient>
</defs>
<rect width="500" height="700" fill="url(#r-sky)"/>
<circle cx="70" cy="90" r="1.8" fill="#fff" opacity="0.9"/>
<circle cx="420" cy="70" r="1.4" fill="#fff" opacity="0.7"/>
<circle cx="160" cy="140" r="1" fill="#fff" opacity="0.6"/>
<circle cx="390" cy="180" r="1.2" fill="#fff" opacity="0.7"/>
<circle cx="250" cy="260" r="220" fill="url(#r-halo)"/>
<rect x="0" y="490" width="500" height="210" fill="url(#r-floor)"/>
<circle cx="250" cy="540" r="110" fill="url(#r-sun)"/>
<g fill="#1d0a3e">
  <rect x="155" y="500" width="190" height="4"/>
  <rect x="150" y="512" width="200" height="5"/>
  <rect x="145" y="526" width="210" height="6"/>
</g>
<g stroke="#ff5da6" stroke-width="1.6" fill="none" opacity="0.9">
  <line x1="0" y1="490" x2="500" y2="490"/>
  <line x1="0" y1="520" x2="500" y2="520"/>
  <line x1="0" y1="560" x2="500" y2="560"/>
  <line x1="0" y1="612" x2="500" y2="612"/>
  <line x1="0" y1="680" x2="500" y2="680"/>
  <line x1="250" y1="490" x2="-180" y2="700"/>
  <line x1="250" y1="490" x2="40" y2="700"/>
  <line x1="250" y1="490" x2="160" y2="700"/>
  <line x1="250" y1="490" x2="220" y2="700"/>
  <line x1="250" y1="490" x2="280" y2="700"/>
  <line x1="250" y1="490" x2="340" y2="700"/>
  <line x1="250" y1="490" x2="460" y2="700"/>
  <line x1="250" y1="490" x2="680" y2="700"/>
</g>
`.trim(),se=`
<defs>
  <radialGradient id="e-bg" cx="50%" cy="42%" r="75%">
    <stop offset="0" stop-color="#ff66d9"/>
    <stop offset="0.45" stop-color="#8a23c9"/>
    <stop offset="1" stop-color="#1a0246"/>
  </radialGradient>
  <radialGradient id="e-spot" cx="50%" cy="40%" r="35%">
    <stop offset="0" stop-color="#ffe27a" stop-opacity="0.7"/>
    <stop offset="0.5" stop-color="#ff66d9" stop-opacity="0.3"/>
    <stop offset="1" stop-color="#ff66d9" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect width="500" height="700" fill="url(#e-bg)"/>
<circle cx="250" cy="290" r="220" fill="url(#e-spot)"/>
<g fill="#ffe27a" opacity="0.5">
  <polygon points="250,320 -80,0 160,0"/>
  <polygon points="250,320 210,0 290,0"/>
  <polygon points="250,320 340,0 580,0"/>
</g>
<g fill="#00e5ff" opacity="0.32">
  <polygon points="250,320 -40,120 -40,0 60,0"/>
  <polygon points="250,320 540,120 540,0 440,0"/>
</g>
<g fill="#fff">
  <circle cx="70" cy="100" r="1.8"/>
  <circle cx="420" cy="70" r="2.2"/>
  <circle cx="150" cy="180" r="1.2"/>
  <circle cx="380" cy="210" r="1.5"/>
  <circle cx="90" cy="260" r="1"/>
  <circle cx="450" cy="280" r="1.3"/>
  <circle cx="220" cy="70" r="1"/>
  <circle cx="330" cy="150" r="1.2"/>
</g>
<g stroke="#fff" stroke-width="0.6" opacity="0.08">
  <line x1="0" y1="120" x2="500" y2="120"/>
  <line x1="0" y1="180" x2="500" y2="180"/>
  <line x1="0" y1="240" x2="500" y2="240"/>
  <line x1="0" y1="300" x2="500" y2="300"/>
  <line x1="0" y1="420" x2="500" y2="420"/>
  <line x1="0" y1="480" x2="500" y2="480"/>
  <line x1="0" y1="540" x2="500" y2="540"/>
  <line x1="0" y1="600" x2="500" y2="600"/>
</g>
`.trim(),ae=`
<defs>
  <linearGradient id="d-bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#3a7fc4"/>
    <stop offset="0.48" stop-color="#2ba3b8"/>
    <stop offset="1" stop-color="#0a2f42"/>
  </linearGradient>
  <radialGradient id="d-halo" cx="50%" cy="42%" r="48%">
    <stop offset="0" stop-color="#b4fff0" stop-opacity="0.85"/>
    <stop offset="0.5" stop-color="#4df5c9" stop-opacity="0.35"/>
    <stop offset="1" stop-color="#4df5c9" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect width="500" height="700" fill="url(#d-bg)"/>
<circle cx="250" cy="290" r="230" fill="url(#d-halo)"/>
<g stroke="#4df5c9" opacity="0.55">
  <line x1="0" y1="150" x2="95" y2="150" stroke-width="1.6"/>
  <line x1="405" y1="140" x2="500" y2="140" stroke-width="1.6"/>
  <line x1="0" y1="210" x2="60" y2="210" stroke-width="1.2"/>
  <line x1="440" y1="220" x2="500" y2="220" stroke-width="1.2"/>
  <line x1="0" y1="460" x2="110" y2="460" stroke-width="1.6"/>
  <line x1="390" y1="470" x2="500" y2="470" stroke-width="1.6"/>
  <line x1="0" y1="510" x2="70" y2="510" stroke-width="1.2"/>
  <line x1="430" y1="520" x2="500" y2="520" stroke-width="1.2"/>
</g>
<g fill="#ffd166">
  <path d="M90 110 l3 -9 l3 9 l9 3 l-9 3 l-3 9 l-3 -9 l-9 -3 z"/>
  <path d="M410 130 l2.5 -8 l2.5 8 l8 2.5 l-8 2.5 l-2.5 8 l-2.5 -8 l-8 -2.5 z"/>
  <path d="M140 560 l2 -6 l2 6 l6 2 l-6 2 l-2 6 l-2 -6 l-6 -2 z"/>
  <path d="M380 590 l2.5 -7 l2.5 7 l7 2.5 l-7 2.5 l-2.5 7 l-2.5 -7 l-7 -2.5 z"/>
</g>
<g fill="#fff" opacity="0.6">
  <circle cx="60" cy="80" r="1"/>
  <circle cx="460" cy="90" r="1.2"/>
  <circle cx="190" cy="60" r="0.8"/>
  <circle cx="340" cy="70" r="1"/>
  <circle cx="70" cy="620" r="1"/>
  <circle cx="450" cy="600" r="0.9"/>
  <circle cx="260" cy="600" r="0.8"/>
</g>
<g fill="none" stroke="#ff4fa3" stroke-width="2.5" stroke-linecap="round" opacity="0.9">
  <polyline points="30,50 30,30 50,30"/>
  <polyline points="470,30 450,30 450,50"/>
  <polyline points="30,670 30,680 50,680"/>
  <polyline points="470,680 450,680 450,670"/>
</g>
`.trim(),oe=`
<defs>
  <linearGradient id="t-bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#1a1b4b"/>
    <stop offset="0.5" stop-color="#0a1540"/>
    <stop offset="1" stop-color="#050924"/>
  </linearGradient>
  <radialGradient id="t-halo" cx="50%" cy="42%" r="55%">
    <stop offset="0" stop-color="#b07bff" stop-opacity="0.55"/>
    <stop offset="0.6" stop-color="#b07bff" stop-opacity="0.15"/>
    <stop offset="1" stop-color="#b07bff" stop-opacity="0"/>
  </radialGradient>
  <pattern id="t-grid" x="0" y="0" width="28" height="28" patternUnits="userSpaceOnUse">
    <path d="M 28 0 L 0 0 0 28" fill="none" stroke="#4b4edb" stroke-width="0.6" opacity="0.35"/>
  </pattern>
</defs>
<rect width="500" height="700" fill="url(#t-bg)"/>
<rect width="500" height="700" fill="url(#t-grid)"/>
<circle cx="250" cy="290" r="240" fill="url(#t-halo)"/>

<g stroke="#ff4bc8" stroke-width="1.4" opacity="0.55" fill="none">
  <line x1="80" y1="120" x2="250" y2="90"/>
  <line x1="250" y1="90" x2="420" y2="130"/>
  <line x1="80" y1="120" x2="100" y2="560"/>
  <line x1="420" y1="130" x2="420" y2="570"/>
  <line x1="100" y1="560" x2="420" y2="570"/>
</g>
<g fill="#ff4bc8">
  <circle cx="80" cy="120" r="4.5"/>
  <circle cx="420" cy="130" r="4.5"/>
  <circle cx="100" cy="560" r="4.5"/>
  <circle cx="420" cy="570" r="4.5"/>
  <circle cx="250" cy="90" r="3.5"/>
</g>
<g fill="#fff" opacity="0.85">
  <circle cx="80" cy="120" r="1.5"/>
  <circle cx="420" cy="130" r="1.5"/>
  <circle cx="100" cy="560" r="1.5"/>
  <circle cx="420" cy="570" r="1.5"/>
</g>

<g fill="#ffd166">
  <path d="M55 320 l2.5 -7 l2.5 7 l7 2.5 l-7 2.5 l-2.5 7 l-2.5 -7 l-7 -2.5 z"/>
  <path d="M440 340 l2 -6 l2 6 l6 2 l-6 2 l-2 6 l-2 -6 l-6 -2 z"/>
  <path d="M200 620 l2 -6 l2 6 l6 2 l-6 2 l-2 6 l-2 -6 l-6 -2 z"/>
</g>

<g stroke="#4df5c9" stroke-width="1.6" opacity="0.85">
  <line x1="20" y1="40" x2="50" y2="40"/>
  <line x1="35" y1="25" x2="35" y2="55"/>
  <line x1="450" y1="40" x2="480" y2="40"/>
  <line x1="465" y1="25" x2="465" y2="55"/>
  <line x1="20" y1="660" x2="50" y2="660"/>
  <line x1="35" y1="645" x2="35" y2="675"/>
  <line x1="450" y1="660" x2="480" y2="660"/>
  <line x1="465" y1="645" x2="465" y2="675"/>
</g>
`.trim(),ne=({title:e,fig:a,iconInner:i,background:r,iconColor:s,iconY:o,iconScale:l,foregroundImage:c,alt:d})=>{const n=p.useRef(null);return te(n),t.jsx("div",{ref:n,className:"sp00ky-scroll-wrap",children:t.jsx(y.Root,{className:"sp00ky-sticker-root",style:{minHeight:"auto","--sticker-card-width":"200px"},children:t.jsx(y.Scene,{children:t.jsxs(y.Card,{children:[t.jsx(y.ImageLayer,{src:re(r,i,s,o,l),alt:d,parallax:!0}),c&&t.jsx(y.ImageLayer,{src:c,alt:d,objectFit:"contain",parallax:!0}),t.jsx(y.Pattern,{textureUrl:"https://assets.codepen.io/605876/figma-texture.png",opacity:.4,mixBlendMode:"multiply",children:t.jsx(y.Refraction,{intensity:1})}),t.jsx(y.Watermark,{imageUrl:"/logo_transparent.svg",opacity:.2,children:t.jsx(y.Refraction,{intensity:1})}),t.jsx(y.Content,{children:t.jsxs("div",{style:{position:"absolute",inset:0,zIndex:2,borderRadius:"8cqi",opacity:1,filter:"url(#hologram-lighting)",clipPath:"inset(0 0 0 0 round 8cqi)"},children:[t.jsx("div",{style:{position:"absolute",inset:"-1px",border:"calc((8cqi * 0.5) + 1px) solid hsl(0 0% 25%)",borderRadius:"8cqi",zIndex:99}}),t.jsx("div",{style:{position:"absolute",width:"calc(8cqi * 4)",bottom:"calc(8cqi * 0.85)",left:"calc(8cqi * 0.65)",zIndex:100},children:t.jsx("img",{src:"/logo.svg",alt:"sp00ky",style:{width:"100%",display:"block"}})})]})}),t.jsx(y.Spotlight,{intensity:1}),t.jsx(y.Glare,{})]})})})})},ce=()=>t.jsx("svg",{className:"sr-only",xmlns:"http://www.w3.org/2000/svg","aria-hidden":"true",children:t.jsxs("defs",{children:[t.jsxs("filter",{id:"hologram-lighting",children:[t.jsx("feGaussianBlur",{in:"SourceAlpha",stdDeviation:"2",result:"blur"}),t.jsx("feSpecularLighting",{result:"lighting",in:"blur",surfaceScale:8,specularConstant:12,specularExponent:120,lightingColor:"hsl(0 0% 6%)",children:t.jsx("fePointLight",{x:50,y:50,z:300})}),t.jsx("feComposite",{in:"lighting",in2:"SourceAlpha",operator:"in",result:"composite"}),t.jsx("feComposite",{in:"SourceGraphic",in2:"composite",operator:"arithmetic",k1:0,k2:1,k3:1,k4:0,result:"litPaint"})]}),t.jsxs("filter",{id:"hologram-sticker",children:[t.jsx("feMorphology",{in:"SourceAlpha",result:"dilate",operator:"dilate",radius:2}),t.jsx("feFlood",{floodColor:"hsl(0 0% 100%)",result:"outlinecolor"}),t.jsx("feComposite",{in:"outlinecolor",in2:"dilate",operator:"in",result:"outlineflat"}),t.jsxs("feMerge",{result:"merged",children:[t.jsx("feMergeNode",{in:"outlineflat"}),t.jsx("feMergeNode",{in:"SourceGraphic"})]})]})]})}),le=[{fig:"0.1",title:"Rust Core",subtitle:"Memory-safe Rust. Zero GC.",iconInner:"",background:ie,iconColor:"#fff6e0",foregroundImage:"/rust3d.png",alt:"Rust"},{fig:"0.2",title:"Instant UI",subtitle:"Optimistic writes. WASM speed.",iconInner:"",background:se,iconColor:"#fff36b",foregroundImage:"/bolt3d.png",alt:"Instant UI"},{fig:"0.3",title:"Job Scheduler",subtitle:"Durable jobs. Zero wiring.",iconInner:"",background:ae,iconColor:"#eafffa",foregroundImage:"/clock3d.png",alt:"Job Scheduler"},{fig:"0.4",title:"Typed Everywhere",subtitle:"One schema. Typed end to end.",iconInner:"",background:oe,iconColor:"#ecfbff",foregroundImage:"/shapes3d.png",alt:"Typed Everywhere"}];function ue(){return t.jsxs(t.Fragment,{children:[t.jsx("div",{className:"mb-24 md:mb-32",children:t.jsx(B,{className:"text-4xl md:text-6xl font-semibold leading-tight tracking-tight",segments:[{text:"It's spooky. ",preRevealed:!0},{text:"Data changes, every screen updates. Reactive queries keep every tab, every device, and every user in sync, instantly."}]})}),t.jsx(ce,{}),t.jsx("style",{children:`
        .sp00ky-scroll-wrap { pointer-events: none; }
        .sp00ky-scroll-wrap .sticker-card { animation: none !important; }
        /* Keep the holographic refraction, spotlight, and parallax ghost alive
           even though hover is disabled, so scroll drives the shimmer. */
        .sp00ky-scroll-wrap .sticker-refraction,
        .sp00ky-scroll-wrap .sticker-spotlight:before {
          opacity: 1 !important;
          transition: none !important;
        }
        .sp00ky-scroll-wrap .sticker-img-layer--parallax img {
          translate:
            calc(var(--sticker-pointer-x) * var(--sticker-parallax-img-x, 5%))
            calc(var(--sticker-pointer-y) * var(--sticker-parallax-img-y, 5%)) !important;
          transition: translate 0.35s cubic-bezier(0.2, 0.8, 0.2, 1) !important;
        }
      `}),t.jsx("div",{className:"grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4",children:le.map((e,a)=>t.jsxs("div",{className:["px-8 py-6",a!==0?"sm:border-l border-white/[0.15]":"",a!==0?"border-t sm:border-t-0 border-white/[0.15]":"",a===2?"sm:border-l-0 md:border-l":""].join(" "),children:[t.jsxs("div",{className:"text-[11px] font-mono text-gray-600 uppercase tracking-wider mb-2",children:["Fig ",e.fig]}),t.jsx("div",{className:"flex items-center justify-center mb-4",children:t.jsx(ne,{title:e.title,fig:e.fig,iconInner:e.iconInner,background:e.background,iconColor:e.iconColor,iconY:e.iconY,iconScale:e.iconScale,foregroundImage:e.foregroundImage,alt:e.alt})}),t.jsx("h3",{className:"text-base font-semibold text-white mb-1",children:e.title}),t.jsx("p",{className:"text-sm text-gray-500",children:e.subtitle})]},e.fig))})]})}function de(){return t.jsxs("div",{className:"p-8 md:p-10 overflow-y-auto h-full",children:[t.jsxs("div",{className:"flex justify-between items-baseline mb-6",children:[t.jsx("h3",{className:"text-lg font-medium tracking-tight text-white",children:"SSP Cluster"}),t.jsx("span",{className:"text-[10px] font-mono font-medium tracking-wider uppercase text-gray-500 bg-white/[0.03] border border-white/[0.15] px-2.5 py-1 rounded-md",children:"Production"})]}),t.jsxs("div",{className:"w-full flex flex-col gap-4 font-mono mb-8",children:[t.jsx("div",{className:"flex justify-center gap-8 relative z-10",children:[{label:"WEB",sub:"React/Vue"},{label:"APP",sub:"Flutter/iOS"},{label:"API",sub:"Backend"}].map(e=>t.jsxs("div",{className:"flex flex-col items-center gap-1",children:[t.jsx("div",{className:"h-8 w-8 border border-white/[0.08] rounded-md bg-white/[0.03] flex items-center justify-center text-gray-500",children:t.jsx("span",{className:"text-[10px]",children:e.label})}),t.jsx("span",{className:"text-[8px] text-gray-600",children:e.sub})]},e.label))}),t.jsxs("div",{className:"border border-dashed border-white/[0.15] p-3 rounded-lg bg-white/[0.02] relative mt-2",children:[t.jsx("div",{className:"absolute -top-2 left-3 bg-[#0a0a0a] px-1.5 text-[8px] text-gray-600 font-medium uppercase tracking-wider",children:"Server Infrastructure"}),t.jsxs("div",{className:"flex flex-col gap-4",children:[t.jsxs("div",{className:"flex gap-2 justify-center items-stretch",children:[t.jsxs("div",{className:"border border-white/[0.15] bg-white/[0.02] p-2 rounded flex flex-col gap-2 flex-1 min-w-0",children:[t.jsxs("div",{className:"flex items-center justify-between border-b border-white/[0.15] pb-1",children:[t.jsx("span",{className:"text-[9px] font-bold text-gray-400",children:"SURREALDB"}),t.jsx("span",{className:"h-1.5 w-1.5 rounded-full bg-gray-500"})]}),t.jsx("div",{className:"space-y-1",children:["Tables & Auth","Live Query Hub","Event Triggers"].map(e=>t.jsx("div",{className:"bg-white/[0.03] border border-white/[0.05] px-1.5 py-1 rounded text-[8px] text-gray-400",children:e},e))})]}),t.jsxs("div",{className:"flex flex-col justify-center items-center gap-0.5 text-[8px] text-gray-600/60 w-8 shrink-0",children:[t.jsx("span",{children:"RPC"}),t.jsx("div",{className:"w-full h-[1px] bg-white/[0.06]"})]}),t.jsxs("div",{className:"border border-white/[0.15] bg-white/[0.02] p-2 rounded flex flex-col gap-2 flex-1 min-w-0",children:[t.jsxs("div",{className:"flex items-center justify-between border-b border-white/[0.15] pb-1",children:[t.jsx("span",{className:"text-[9px] font-bold text-gray-400",children:"SCHEDULER"}),t.jsx("span",{className:"h-1.5 w-1.5 rounded-full bg-gray-500"})]}),t.jsx("div",{className:"space-y-1",children:["Snapshot Replica","WAL","Load Balancer","Health Monitor","Job Scheduler"].map(e=>t.jsx("div",{className:"bg-white/[0.03] border border-white/[0.05] px-1.5 py-0.5 rounded text-[8px] text-gray-400",children:e},e))})]})]}),t.jsx("div",{className:"flex flex-col gap-1.5",children:[{name:"SSP-1",status:"Active"},{name:"SSP-2",status:"Active"},{name:"SSP-3",status:"Bootstrapping"}].map(e=>t.jsxs("div",{className:"border border-white/[0.15] bg-white/[0.02] p-1.5 rounded flex items-center justify-between",children:[t.jsxs("div",{className:"flex items-center gap-1.5",children:[t.jsxs("svg",{className:"w-2.5 h-2.5 flex-shrink-0 text-gray-400",fill:"none",stroke:"currentColor",viewBox:"0 0 24 24",strokeWidth:"2",strokeLinecap:"round",strokeLinejoin:"round",children:[t.jsx("rect",{x:"2",y:"2",width:"20",height:"8",rx:"2",ry:"2"}),t.jsx("rect",{x:"2",y:"14",width:"20",height:"8",rx:"2",ry:"2"})]}),t.jsx("span",{className:"text-[9px] font-bold text-gray-400",children:e.name}),t.jsx("span",{className:"h-1 w-1 rounded-full bg-gray-500"})]}),t.jsx("span",{className:"text-[8px] text-gray-500",children:e.status})]},e.name))})]})]})]}),t.jsxs("p",{className:"text-[11px] text-gray-500/80 mb-4 font-mono leading-relaxed",children:["The Scheduler distributes queries across multiple SSP instances using a persistent RocksDB snapshot replica and WAL for crash recovery. Automatic load balancing and health monitoring ensure ",t.jsx("span",{className:"text-gray-300",children:"zero-downtime deployment"})," and horizontal scalability for enterprise workloads."]}),t.jsx("ul",{className:"space-y-2 font-mono text-xs text-gray-500 border-t border-white/[0.15] pt-4",children:["Horizontal Scaling (Add/Remove SSPs).","Zero-Downtime Deployments.","Intelligent Query Routing & Load Balancing."].map(e=>t.jsxs("li",{className:"flex items-start gap-2 transition-colors duration-300 hover:text-gray-400",children:[t.jsx("svg",{className:"w-4 h-4 text-gray-600 mt-0.5 flex-shrink-0",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:t.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:"2",d:"M5 13l4 4L19 7"})}),t.jsx("span",{children:e})]},e))})]})}function me(){const[e,a]=p.useState(!1),i=p.useCallback(()=>a(!1),[]);return p.useEffect(()=>{if(!e)return;const r=s=>{s.key==="Escape"&&i()};return document.addEventListener("keydown",r),document.body.style.overflow="hidden",()=>{document.removeEventListener("keydown",r),document.body.style.overflow=""}},[e,i]),t.jsxs(t.Fragment,{children:[t.jsx("div",{className:"mt-16 max-w-3xl",children:t.jsx(B,{className:"text-2xl md:text-3xl font-semibold leading-snug",segments:[{text:"Horizontally Scalable. ",preRevealed:!0},{text:"The Scheduler distributes queries across SSP instances with automatic load balancing and zero-downtime deployments."}],trailing:t.jsxs("button",{onClick:()=>a(!0),className:"text-gray-500 hover:text-gray-300 transition-colors duration-200 inline-flex items-center gap-1",children:["Learn more",t.jsx("svg",{className:"w-5 h-5 inline",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",strokeWidth:"2",children:t.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",d:"M13 7l5 5m0 0l-5 5m5-5H6"})})]})})}),typeof document<"u"&&U.createPortal(t.jsxs("div",{className:`fixed inset-0 z-50 transition-opacity duration-300 ${e?"opacity-100 pointer-events-auto":"opacity-0 pointer-events-none"}`,children:[t.jsx("div",{className:"absolute inset-0 bg-black/60 backdrop-blur-sm",onClick:i}),t.jsxs("div",{className:`absolute right-0 top-0 bottom-0 w-full max-w-2xl bg-[#0a0a0a] border-l border-white/[0.15] transition-transform duration-300 ${e?"translate-x-0":"translate-x-full"}`,children:[t.jsx("button",{onClick:i,className:"absolute top-4 right-4 z-10 text-gray-500 hover:text-gray-300 transition-colors",children:t.jsx("svg",{className:"w-5 h-5",fill:"none",viewBox:"0 0 24 24",stroke:"currentColor",children:t.jsx("path",{strokeLinecap:"round",strokeLinejoin:"round",strokeWidth:"2",d:"M6 18L18 6M6 6l12 12"})})}),t.jsx(de,{})]})]}),document.body)]})}export{ue as FeatureGrid,me as ScalableText};
