/** @type {import('tailwindcss').Config} */
export default {
  content: ["./src/**/*.{astro,html,js,jsx,md,mdx,svelte,ts,tsx,vue}"],
  theme: {
    extend: {
      colors: {
        primary: {
          50: "#fef2f4",
          100: "#fde6e9",
          200: "#faccd6",
          300: "#f7a3b4",
          400: "#f27089",
          500: "#E94560", // Main accent color
          600: "#d63852",
          700: "#b82a43",
          800: "#99253e",
          900: "#80223a",
          950: "#470e1b",
        },
        secondary: {
          50: "#f5f3ff",
          100: "#ede9ff",
          200: "#ddd6ff",
          300: "#c4b5ff",
          400: "#a689ff",
          500: "#8a59ff",
          600: "#7c34f7",
          700: "#6d22e3",
          800: "#5b1dbf",
          900: "#533483", // Purple accent
          950: "#2e1054",
        },
        navy: {
          50: "#f0f4f8",
          100: "#dae3ed",
          200: "#b8cade",
          300: "#8da9c8",
          400: "#5e84ae",
          500: "#426796",
          600: "#33527d",
          700: "#2a4165",
          800: "#263855",
          900: "#0F3460", // Dark blue
          950: "#0a1f3d",
        },
        deepNavy: {
          DEFAULT: "#0a0e1a", // Very dark, almost black
          light: "#0f1520",
          dark: "#050810",
        },
        paper: "#eeeeee", // Off-white for terminals/sections
        // Docs Theme
        background: '#09090b', // Almost black
        surface: {
          DEFAULT: '#0a0a0a',
          elevated: '#121212',
          border: '#1a1a1a',
        },
        text: {
          main: '#f4f4f5',     // High contrast text
          muted: '#a1a1aa',    // Secondary text
        },
        border: '#27272a',     // Subtle borders
        // Glow variations for accents
        glow: {
          purple: 'rgba(168, 85, 247, 0.15)',
          blue: 'rgba(59, 130, 246, 0.15)',
          green: 'rgba(34, 197, 94, 0.15)',
          white: 'rgba(255, 255, 255, 0.3)',
        },
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "sans-serif"],
      },
      fontSize: {
        'hero': ['clamp(2.5rem, 5vw, 4.5rem)', { lineHeight: '1.1', letterSpacing: '-0.02em' }],
        'section': ['clamp(2rem, 4vw, 3rem)', { lineHeight: '1.2', letterSpacing: '-0.01em' }],
        'card-title': ['clamp(1.25rem, 2vw, 1.5rem)', { lineHeight: '1.3' }],
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
