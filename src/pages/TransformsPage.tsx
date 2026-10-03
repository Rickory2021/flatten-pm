// src/pages/TransformsPage.tsx
import { ArrowRightLeft } from "lucide-react";

export function TransformsPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <ArrowRightLeft className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Transforms</h1>
      </div>
      <p className="text-text-muted mt-1">
        File and directory transforms (JavaScript, QuickJS-NG).
      </p>
    </div>
  );
}
