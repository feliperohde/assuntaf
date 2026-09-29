import { invoke } from '@tauri-apps/api/core';
import { useSyncExternalStore } from 'react';

/**
 * Plays one transcript line from the meeting's recording. Only one clip plays
 * at a time; starting another (or pressing the same one) stops the current.
 * Uses Web Audio, so no media URL or asset-protocol access is needed.
 */

type State = { key: string | null; loading: boolean };

let state: State = { key: null, loading: false };
let context: AudioContext | null = null;
let source: AudioBufferSourceNode | null = null;
let request = 0;
const listeners = new Set<() => void>();

function set(next: State) {
  state = next;
  listeners.forEach(listener => listener());
}

function stopSource() {
  if (source) {
    source.onended = null;
    try {
      source.stop();
    } catch {
      // already stopped
    }
    source = null;
  }
}

export function stopClip() {
  request++;
  stopSource();
  set({ key: null, loading: false });
}

export async function toggleClip(meetingId: string, key: string, start: number, end?: number) {
  if (state.key === key) {
    stopClip();
    return;
  }
  stopSource();
  const mine = ++request;
  set({ key, loading: true });
  try {
    const bytes = await invoke<ArrayBuffer>('get_meeting_audio_clip', { meetingId, start, end: end ?? null });
    if (mine !== request) return;
    context ??= new AudioContext();
    if (context.state === 'suspended') await context.resume();
    const buffer = await context.decodeAudioData(bytes instanceof ArrayBuffer ? bytes : new Uint8Array(bytes as any).buffer);
    if (mine !== request) return;
    const node = context.createBufferSource();
    node.buffer = buffer;
    node.connect(context.destination);
    node.onended = () => {
      if (mine === request) set({ key: null, loading: false });
    };
    source = node;
    node.start();
    set({ key, loading: false });
  } catch (error) {
    if (mine === request) set({ key: null, loading: false });
    throw error;
  }
}

export function useClipState(): State {
  return useSyncExternalStore(
    listener => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => state,
    () => state,
  );
}
