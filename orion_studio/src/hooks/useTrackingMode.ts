import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { GatewayConnection } from "../lib/gateway";
import { TrackingPreference, type TrackingState } from "../lib/trackingPreference";

const disconnected: TrackingState = { mode: null, cameraEnabled: null, pending: false, notice: "", error: null };
const noSubscription = () => () => {};
const disconnectedSnapshot = () => disconnected;
export function useTrackingMode(connection: GatewayConnection | null, supported: boolean) {
  const preference = useMemo(() => connection && supported ? new TrackingPreference(connection) : null, [connection, supported]);
  useEffect(() => { preference?.start(); return () => preference?.stop(); }, [preference]);
  const state = useSyncExternalStore(preference?.subscribe ?? noSubscription,
    preference?.snapshot ?? disconnectedSnapshot, preference?.snapshot ?? disconnectedSnapshot);
  return { ...state, choose: preference?.choose, cameraChanged: preference?.cameraChanged };
}
