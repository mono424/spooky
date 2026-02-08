import { mergeProps } from "solid-js";

export default function SlickButton(p: { loading: boolean; loadingLabel?: string, onClick: () => void; label?: string, children?: any }) {
  return (
    <>
      <style>
        {`
        /* 1. RGB Gradient Flow (Border) */
        @keyframes border-flow {
          0% { background-position: 0% 50%; }
          50% { background-position: 100% 50%; }
          100% { background-position: 0% 50%; }
        }
        
        /* 2. Cyberpunk Loading Stripes */
        @keyframes stripe-slide {
          0% { background-position: 0 0; }
          100% { background-position: 56px 0; } /* 56px = 2x pattern size */
        }
        
        .bg-stripes-anim {
          background-image: repeating-linear-gradient(
            -45deg,
            rgba(255, 255, 255, 0.03) 0,
            rgba(255, 255, 255, 0.03) 20px,
            transparent 20px,
            transparent 40px
          );
          background-size: 200% 200%;
          animation: stripe-slide 1s linear infinite;
        }

        .animate-border-flow {
          background-size: 200% 200%;
          animation: border-flow 3s ease infinite;
        }
        `}
      </style>

      {/* OUTER WRAPPER: The Gradient Border 
        - Acts as the "border" via p-[1px]
        - Handles the glow effect
      */}
      <button
        onClick={p.onClick}
        disabled={p.loading}
        class="
          group relative inline-flex p-[1px] rounded-lg shadow-lg
          bg-gradient-to-r from-cyan-500 via-purple-500 to-pink-500
          animate-border-flow
          transition-all duration-300 ease-out
          hover:shadow-[0_0_15px_rgba(168,85,247,0.5)] hover:scale-[1.01]
          active:scale-[0.98]
          disabled:opacity-80 disabled:cursor-wait disabled:hover:scale-100 disabled:shadow-none
        "
      >
        {/* INNER CONTAINER: The Button Body 
          - Handles background color (Black vs Stripes)
          - Handles hover fill effects
        */}
        <div
          class="
            relative w-full h-7 min-w-[140px] px-6 rounded-[7px]
            flex items-center justify-center
            transition-all duration-300
            bg-black
          "
          classList={{
            // On hover (not loading), lighten background slightly for 'slick' feel
            "group-hover:bg-neutral-900": !p.loading,
            // On loading, use transparent black so stripes show, or add texture
            "bg-stripes-anim bg-neutral-900": p.loading
          }}
        >
          
          {/* CONTENT STACK: 
            CSS Grid with 1 cell allows us to stack Text and Spinner 
            perfectly on top of each other without absolute positioning issues.
          */}
          <div class="grid place-items-center" style={{ "grid-template-areas": "'stack'" }}>
            
            {/* 1. THE LABEL (Fades Out) */}
            <span
              class="
                font-mono text-[10px] font-semibold tracking-[0.15em] text-white uppercase
                transition-all duration-300 transform
              "
              style={{ "grid-area": "stack" }}
              classList={{
                "opacity-100 scale-100 blur-0 translate-y-0": !p.loading,
                "opacity-0 scale-95 blur-sm translate-y-2": p.loading // Slide down effect
              }}
            >
              {p.children || p.label}
            </span>

            {/* 2. THE SPINNER (Fades In) */}
            <div
              class="flex items-center gap-2 transition-all duration-300 transform"
              style={{ "grid-area": "stack" }}
              classList={{
                "opacity-0 scale-75 -translate-y-2": !p.loading,
                "opacity-100 scale-100 translate-y-0": p.loading // Slide in effect
              }}
            >
              <svg class="animate-spin h-4 w-4 text-cyan-400" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
              </svg>
              <span class="font-mono text-[10px] text-cyan-400 font-bold tracking-widest">
                {p.loadingLabel || 'LOADING'}
              </span>
            </div>
            
          </div>
        </div>
      </button>
    </>
  );
}