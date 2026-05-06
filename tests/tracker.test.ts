import { describe, it, expect } from 'vitest';
import { EMPTY_TRACKER_STATE, pushTrackerSample, resetTrackerState } from '../src/tracker.ts';

describe('EMPTY_TRACKER_STATE', () => {
  it('starts with no current cell and an empty trail', () => {
    expect(EMPTY_TRACKER_STATE.current).toBeNull();
    expect(EMPTY_TRACKER_STATE.trail).toHaveLength(0);
  });

  it('is frozen', () => {
    expect(Object.isFrozen(EMPTY_TRACKER_STATE)).toBe(true);
  });
});

describe('resetTrackerState', () => {
  it('returns a fresh empty state', () => {
    const r = resetTrackerState();
    expect(r.trail).toHaveLength(0);
    expect(r.current).toBeNull();
  });

  it('returns a new mutable trail array each call', () => {
    const a = resetTrackerState();
    const b = resetTrackerState();
    expect(a.trail).not.toBe(b.trail);
  });
});

describe('pushTrackerSample', () => {
  it('appends the first sample to the trail and sets current', () => {
    const r = pushTrackerSample(EMPTY_TRACKER_STATE, { x: 1, y: 2, z: 3, energy: 10 }, 60);
    expect(r.current).toEqual({ x: 1, y: 2, z: 3, energy: 10 });
    expect(r.trail).toEqual([{ x: 1, y: 2, z: 3, energy: 10 }]);
  });

  it('appends a new entry on a position change in x', () => {
    const a = pushTrackerSample(EMPTY_TRACKER_STATE, { x: 0, y: 0, z: 0, energy: 5 }, 60);
    const b = pushTrackerSample(a, { x: 1, y: 0, z: 0, energy: 6 }, 60);
    expect(b.trail).toHaveLength(2);
    expect(b.trail[1]).toEqual({ x: 1, y: 0, z: 0, energy: 6 });
  });

  it('appends a new entry on a position change in y', () => {
    const a = pushTrackerSample(EMPTY_TRACKER_STATE, { x: 0, y: 0, z: 0, energy: 5 }, 60);
    const b = pushTrackerSample(a, { x: 0, y: 1, z: 0, energy: 6 }, 60);
    expect(b.trail).toHaveLength(2);
    expect(b.trail[1]).toEqual({ x: 0, y: 1, z: 0, energy: 6 });
  });

  it('appends a new entry on a position change in z', () => {
    const a = pushTrackerSample(EMPTY_TRACKER_STATE, { x: 0, y: 0, z: 0, energy: 5 }, 60);
    const b = pushTrackerSample(a, { x: 0, y: 0, z: 1, energy: 6 }, 60);
    expect(b.trail).toHaveLength(2);
    expect(b.trail[1]).toEqual({ x: 0, y: 0, z: 1, energy: 6 });
  });

  it('updates the energy on the trailing entry when position is unchanged', () => {
    const a = pushTrackerSample(EMPTY_TRACKER_STATE, { x: 1, y: 1, z: 1, energy: 5 }, 60);
    const b = pushTrackerSample(a, { x: 1, y: 1, z: 1, energy: 9 }, 60);
    expect(b.trail).toHaveLength(1);
    expect(b.trail[0]).toEqual({ x: 1, y: 1, z: 1, energy: 9 });
    expect(b.current?.energy).toBe(9);
  });

  it('caps the trail at trailLen + 1 entries', () => {
    let s = EMPTY_TRACKER_STATE;
    for (let i = 0; i < 10; i += 1) {
      s = pushTrackerSample(s, { x: i, y: 0, z: 0, energy: i }, 3);
    }
    expect(s.trail).toHaveLength(4); // trailLen=3 → cap=4
    expect(s.trail.map((c) => c.x)).toEqual([6, 7, 8, 9]);
  });

  it('treats trailLen of 0 as "current sample only"', () => {
    let s = EMPTY_TRACKER_STATE;
    s = pushTrackerSample(s, { x: 0, y: 0, z: 0, energy: 1 }, 0);
    s = pushTrackerSample(s, { x: 1, y: 0, z: 0, energy: 2 }, 0);
    s = pushTrackerSample(s, { x: 2, y: 0, z: 0, energy: 3 }, 0);
    expect(s.trail).toHaveLength(1);
    expect(s.trail[0]?.x).toBe(2);
  });

  it('clamps a negative trailLen to 0 (treated as cap=1)', () => {
    let s = EMPTY_TRACKER_STATE;
    s = pushTrackerSample(s, { x: 0, y: 0, z: 0, energy: 1 }, -5);
    s = pushTrackerSample(s, { x: 1, y: 0, z: 0, energy: 2 }, -5);
    expect(s.trail).toHaveLength(1);
    expect(s.trail[0]?.x).toBe(1);
  });

  it('does not mutate the input state', () => {
    const a = pushTrackerSample(EMPTY_TRACKER_STATE, { x: 0, y: 0, z: 0, energy: 1 }, 60);
    const trailBefore = a.trail.slice();
    pushTrackerSample(a, { x: 1, y: 0, z: 0, energy: 2 }, 60);
    expect(a.trail).toEqual(trailBefore);
  });
});
