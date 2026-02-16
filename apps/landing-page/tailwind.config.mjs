/** @type {import('tailwindcss').Config} */
export default {
  content: ["./src/**/*.{astro,html,js,jsx,md,mdx,svelte,ts,tsx,vue}"],
  theme: {
    extend: {
      colors: {
        // Professional Brand Colors (Purple)
        brand: {
          50: "#f5f3ff",
          100: "#ede9ff",
          200: "#ddd6ff",
          300: "#c4b5ff",
          400: "#a689ff",
          500: "#8a59ff", // Primary brand color
          600: "#7c34f7",
          700: "#6d22e3",
          800: "#5b1dbf",
          900: "#533483",
          950: "#2e1054",
        },
        // Accent Colors (Green)
        accent: {
          50: "#f0fdf4",
          100: "#dcfce7",
          200: "#bbf7d0",
          300: "#86efac",
          400: "#4ade80",
          500: "#22c55e", // Success/active states
          600: "#16a34a",
          700: "#15803d",
          800: "#166534",
          900: "#14532d",
          950: "#052e16",
        },
        // Enterprise Colors (Orange)
        orange: {
          50: "#fff7ed",
          100: "#ffedd5",
          200: "#fed7aa",
          300: "#fdba74",
          400: "#fb923c",
          500: "#f97316", // Primary orange
          600: "#ea580c",
          700: "#c2410c",
          800: "#9a3412",
          900: "#7c2d12",
          950: "#431407",
        },
        // Background & Surface
        background: '#09090b',
        surface: {
          DEFAULT: '#0a0a0a',
          elevated: '#121212',
          hover: '#1a1a1a',
          border: '#27272a',
        },
        // Text Hierarchy
        text: {
          primary: '#ffffff',
          secondary: '#f4f4f5',
          tertiary: '#a1a1aa',
          muted: '#71717a',
        },
        // Legacy colors for backwards compatibility
        border: '#27272a',
        paper: "#eeeeee",
      },
      fontFamily: {
        sans: ['Inter', '-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'Roboto', 'Helvetica Neue', 'Arial', 'sans-serif'],
        mono: ['JetBrains Mono', 'Menlo', 'Monaco', 'Consolas', 'Liberation Mono', 'Courier New', 'monospace'],
      },
      fontSize: {
        // Hero & Section Headers
        'hero': ['clamp(2.5rem, 5vw, 4.5rem)', { lineHeight: '1.1', letterSpacing: '-0.02em' }],
        'section': ['clamp(2rem, 4vw, 3rem)', { lineHeight: '1.2', letterSpacing: '-0.01em' }],
        'card-title': ['clamp(1.25rem, 2vw, 1.5rem)', { lineHeight: '1.3' }],
        // Body Text (Professional & Readable)
        'body-lg': ['1.125rem', { lineHeight: '1.75', letterSpacing: '-0.01em' }], // 18px
        'body': ['1rem', { lineHeight: '1.625', letterSpacing: '0' }],              // 16px
        'body-sm': ['0.875rem', { lineHeight: '1.5', letterSpacing: '0' }],        // 14px
      },
      animation: {
        "fade-in": "fadeIn 0.5s ease-in-out",
        "fade-in-fast": "fadeIn 0.3s ease-in-out",
        "slide-up": "slideUp 0.5s ease-out",
        "slide-down": "slideDown 0.5s ease-out",
        "scale-in": "scaleIn 0.4s ease-out",
        "glow-pulse": "glowPulse 2s ease-in-out infinite",
        "gradient": "gradient 8s ease infinite",
        float: "float 3s ease-in-out infinite",
      },
      keyframes: {
        fadeIn: {
          "0%": { opacity: "0" },
          "100%": { opacity: "1" },
        },
        slideUp: {
          "0%": { transform: "translateY(20px)", opacity: "0" },
          "100%": { transform: "translateY(0)", opacity: "1" },
        },
        slideDown: {
          "0%": { transform: "translateY(-20px)", opacity: "0" },
          "100%": { transform: "translateY(0)", opacity: "1" },
        },
        scaleIn: {
          "0%": { transform: "scale(0.95)", opacity: "0" },
          "100%": { transform: "scale(1)", opacity: "1" },
        },
        glowPulse: {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: "0.5" },
        },
        gradient: {
          "0%, 100%": { backgroundPosition: "0% 50%" },
          "50%": { backgroundPosition: "100% 50%" },
        },
        float: {
          "0%, 100%": { transform: "translateY(0px)" },
          "50%": { transform: "translateY(-10px)" },
        },
      },
      transitionDuration: {
        '400': '400ms',
      },
      typography: (theme) => ({
        DEFAULT: {
          css: {
            color: theme('colors.text.muted'),
            maxWidth: 'none',
            h1: { color: theme('colors.text.main'), fontWeight: '600' },
            h2: { color: theme('colors.text.main'), fontWeight: '500', marginTop: '2em' },
            h3: { color: theme('colors.text.main'), fontWeight: '500' },
            strong: { color: theme('colors.text.main') },
            code: {
              color: theme('colors.text.main'),
              backgroundColor: theme('colors.surface'),
              padding: '0.2em 0.4em',
              borderRadius: '0.25rem',
              fontWeight: '400',
            },
            'code::before': { content: '""' },
            'code::after': { content: '""' },
            pre: {
              backgroundColor: '#121212', // Darker code block bg
              border: `1px solid ${theme('colors.border')}`,
              borderRadius: '0.5rem',
            },
          },
        },
      }),
    },
  },
  plugins: [require('@tailwindcss/typography')],
};
