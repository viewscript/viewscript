/**
 * Counter Logic Module
 *
 * Pure TypeScript state management for the counter demo.
 * This module exports functions that will be called from click handlers.
 *
 * In a future release, these exports can be bound to .vs files via FFI:
 *   q bind count = getCount()
 *   on Button.click -> increment()
 */

let count = 0;

/**
 * Increment the counter and return the new value.
 */
export function increment(): number {
  return ++count;
}

/**
 * Decrement the counter and return the new value.
 */
export function decrement(): number {
  return --count;
}

/**
 * Get the current counter value.
 */
export function getCount(): number {
  return count;
}

/**
 * Reset the counter to zero.
 */
export function reset(): number {
  count = 0;
  return count;
}
