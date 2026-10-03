// src/components/layout/Layout.tsx
import { Outlet } from "react-router";
import {
  Group,
  Panel,
  Separator,
  useDefaultLayout,
} from "react-resizable-panels";
import { Sidebar } from "./Sidebar";
import { WatchDock } from "./WatchDock";

export function Layout() {
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: "main-layout",
    storage: localStorage,
  });

  return (
    <div className="flex h-screen bg-bg text-text">
      <Sidebar />
      <div className="flex-1 min-w-0 h-full">
        <Group
          defaultLayout={defaultLayout}
          onLayoutChanged={onLayoutChanged}
          orientation="vertical"
        >
          <Panel defaultSize="100%" minSize="30%">
            <div className="h-full overflow-auto">
              <Outlet />
            </div>
          </Panel>
          <Separator className="h-1.5 bg-border hover:bg-accent transition-colors cursor-row-resize" />
          <Panel collapsible defaultSize="0%" minSize="10%">
            <WatchDock />
          </Panel>
        </Group>
      </div>
    </div>
  );
}
