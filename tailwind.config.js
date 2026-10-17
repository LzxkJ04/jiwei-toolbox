/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,vue}"],
  darkMode: ["selector", '[data-theme="dark"]'],
  theme: {
    extend: {
      colors: {
        background: "var(--bg)",
        foreground: "var(--text)",
        card: {
          DEFAULT: "var(--card)",
          border: "var(--card-border)",
        },
        primary: {
          DEFAULT: "var(--primary)",
          hover: "var(--primary-hov)",
          foreground: "#ffffff",
        },
        "primary-soft": "var(--primary-soft)",
        muted: {
          DEFAULT: "var(--btn-gray)",
          foreground: "var(--muted)",
        },
        destructive: {
          DEFAULT: "#e5484d",
          foreground: "#ffffff",
        },
        success: "#30a46c",
        warning: "#f59e0b",
      },
      borderRadius: {
        xl2: "1rem",
      },
      boxShadow: {
        glass: "0 8px 32px rgb(0 0 0 / 0.18)",
        "glass-hover": "0 14px 40px rgb(0 0 0 / 0.28)",
      },
      width: {
        sidebar: "220px",
        "sidebar-collapsed": "64px",
      },
    },
  },
  plugins: [],
};
