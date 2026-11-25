// Before anything is drawn, so a chosen theme never flashes. The system's own needs no attribute.
try { const t = localStorage.getItem("famulen-theme"); if (t === "light" || t === "dark") document.documentElement.dataset.theme = t; else if (t === "system") delete document.documentElement.dataset.theme; } catch { /* the system's */ }
