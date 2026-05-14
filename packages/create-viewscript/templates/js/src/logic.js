/**
 * Counter Logic Module
 *
 * Pure JavaScript state management for the counter demo.
 * This module exports functions that will be called from click handlers.
 *
 * In a future release, these exports can be bound to .vs files via FFI:
 *   q bind count = getCount()
 *   on Button.click -> increment()
 */

let count = 0;

/**
 * Increment the counter and return the new value.
 * @returns {number}
 */
export function increment() {
  return ++count;
}

/**
 * Decrement the counter and return the new value.
 * @returns {number}
 */
export function decrement() {
  return --count;
}

/**
 * Get the current counter value.
 * @returns {number}
 */
export function getCount() {
  return count;
}

/**
 * Reset the counter to zero.
 * @returns {number}
 */
export function reset() {
  count = 0;
  return count;
}
