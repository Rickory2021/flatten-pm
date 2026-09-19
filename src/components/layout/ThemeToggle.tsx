// src/components/layout/ThemeToggle.tsx
import { useState } from "react";
import { Sun, Moon } from "lucide-react";

export function ThemeToggle() {
  const [isDark, setIsDark] = useState(
    () => document.documentElement.classList.contains("dark")
  );

  function toggle() {
    const next = isDark ? "light" : "dark";
    localStorage.setItem("flatten-theme", next);
    document.documentElement.classList.toggle("dark", next === "dark");
    setIsDark(next === "dark");
  }

  return (
    <button
      onClick={toggle}
      className="p-2 rounded-md text-text-muted hover:text-text hover:bg-bg transition-colors"
      aria-label={isDark ? "Switch to light mode" : "Switch to dark mode"}
    >
      {isDark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
    </button>
  );
}
