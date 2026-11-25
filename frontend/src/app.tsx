import { MetaProvider } from "@solidjs/meta";
import { Router } from "@solidjs/router";
import { FileRoutes } from "@solidjs/start/router";
import { Suspense } from "solid-js";
import Shell from "./components/Shell";
import { OwnerProvider } from "./lib/owner";
import { WorkgroupsProvider } from "./lib/workgroups";
import "./app.css";

export default function App() {
  return (
    <Router
      root={(props) => (
        <MetaProvider>
          <Suspense>
            <OwnerProvider>
              <WorkgroupsProvider>
                <Shell>{props.children}</Shell>
              </WorkgroupsProvider>
            </OwnerProvider>
          </Suspense>
        </MetaProvider>
      )}
    >
      <FileRoutes />
    </Router>
  );
}
