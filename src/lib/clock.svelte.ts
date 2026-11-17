// The current time, ticking every few seconds so "12 s ago" texts stay right
// without asking the back end for anything.

class Clock {
  /** Unix seconds. */
  now = $state(Math.floor(Date.now() / 1000));

  /** Returns a function that stops the ticking. */
  start(): () => void {
    const tick = () => (this.now = Math.floor(Date.now() / 1000));
    tick();
    const timer = setInterval(tick, 5000);
    return () => clearInterval(timer);
  }
}

export const clock = new Clock();
