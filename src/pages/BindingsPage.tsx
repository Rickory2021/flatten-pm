// src/pages/BindingsPage.tsx
import { Cable } from "lucide-react";

export function BindingsPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <Cable className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Bindings</h1>
      </div>
      <p className="text-text-muted mt-1">
        Recipe-to-repo bindings with argument values.
      </p>
    </div>
  );
}
