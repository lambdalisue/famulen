/**
 * Whether `hostname`, as `location.hostname` gives it, names the machine it is
 * opened on, so that a phone told it would reach the phone itself.
 */
export function reachesOnlyThisMachine(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  return (
    host === "localhost" ||
    host.endsWith(".localhost") ||
    /^127(\.\d+){3}$/.test(host) ||
    host === "0.0.0.0" ||
    host === "[::1]" ||
    host === "[::]"
  );
}
