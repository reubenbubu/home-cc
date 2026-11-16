// The app's modules. Each one is a screen in the sidebar.
// To add a module: add an entry here and a folder under src/routes/.

export interface Module {
  id: string;
  label: string;
  href: string;
  /** SVG path data for a 24x24 stroke icon. */
  icon: string;
}

export const modules: Module[] = [
  { id: "overview", label: "Overview", href: "/", icon: "M3 3h7v9H3zM14 3h7v5h-7zM14 12h7v9h-7zM3 16h7v5H3z" },
  { id: "network", label: "Network", href: "/network", icon: "M12 20h.01M8.5 16.5a5 5 0 0 1 7 0M5 13a10 10 0 0 1 14 0M2 9.5a15 15 0 0 1 20 0" },
  { id: "machines", label: "Machines", href: "/machines", icon: "M3 4h18v12H3zM8 20h8M12 16v4" },
  { id: "docker", label: "Docker", href: "/docker", icon: "M21 8l-9-5-9 5 9 5 9-5zM3 8v8l9 5 9-5V8M12 13v8" },
  { id: "proxmox", label: "Proxmox", href: "/proxmox", icon: "M4 4h16v6H4zM4 14h16v6H4zM8 7h.01M8 17h.01" },
  { id: "terminals", label: "Terminals", href: "/terminals", icon: "M4 17l6-5-6-5M12 19h8" },
];
