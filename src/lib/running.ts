// Keeps one job at a time going for each thing (a reading of one machine, say),
// and lets other code wait for the job that is under way. Kept apart from the
// stores so it can be tested without a browser (running.test.mjs).

export class OneAtATime<K> {
  #running = new Map<K, Promise<void>>();

  /** Starts the job, unless one for the same key is still going; then nothing new is started. Either
   *  way the promise ends when the job that is going has ended. Only the caller that started a job
   *  hears about its failure. */
  run(key: K, job: () => Promise<void>): Promise<void> {
    const going = this.#running.get(key);
    if (going) return going;
    const started = job();
    // What the others wait on never fails: they only want to know when it is over.
    const over = started
      .then(
        () => {},
        () => {},
      )
      .then(() => {
        if (this.#running.get(key) === over) this.#running.delete(key);
      });
    this.#running.set(key, over);
    return started;
  }

  /** Ends when the job going for this key at this moment has ended; at once when there is none. */
  settled(key: K): Promise<void> {
    return this.#running.get(key) ?? Promise.resolve();
  }

  has(key: K): boolean {
    return this.#running.has(key);
  }
}
