// src/pages/DevToolsPage.tsx
import { Terminal } from "lucide-react";

export function DevToolsPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <Terminal className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Dev Tools</h1>
      </div>
      <p className="text-text-muted mt-1">
        Database inspector and query runner.
      </p>
    </div>
  );
}
