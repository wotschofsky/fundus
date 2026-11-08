import { upload, verified } from './upload';

const isAudio = (file: File) =>
  file.type.startsWith('audio/') || /\.(wav|flac|mp3|m4a|aac|ogg|opus)$/i.test(file.name);

/** Uploads started in the browser, sent one batch after another, for the bar at the top. */
class Uploads {
  /** Bytes queued since the bar was last empty, and how many of them have been sent. */
  total = $state(0);
  sent = $state(0);
  /** Files queued since then, and the 1-based one being sent or checked. */
  count = $state(0);
  index = $state(0);
  name = $state('');
  checking = $state(false);
  problems = $state<string[]>([]);
  #queue = Promise.resolve();
  #batches = 0;

  /** Queues the audio among `files`. Resolves, never rejects, once the server has checked them. */
  add(files: File[]) {
    if (!this.#batches) this.problems = [];
    for (const file of files) {
      if (!isAudio(file)) this.problems.push(`${file.name} is not an audio file.`);
      else if (!file.size) this.problems.push(`${file.name} is empty.`);
    }
    files = files.filter((file) => isAudio(file) && file.size);
    if (!files.length) return Promise.resolve();
    const size = files.reduce((sum, file) => sum + file.size, 0);
    const first = this.count;
    this.total += size;
    this.count += files.length;
    this.#batches++;
    // ponytail: batches wait for the previous one's check too; overlap them if checks get slow.
    this.#queue = this.#queue.then(async () => {
      const start = this.sent;
      let current = files[0].name;
      try {
        const id = await upload(files, (sent, i) => {
          this.sent = start + sent;
          this.index = first + i + 1;
          this.name = current = files[i].name;
        });
        this.sent = start + size;
        this.checking = true;
        const result = await verified(id);
        if (result.state === 'failed') {
          this.problems.push(
            `Your vault could not process ${files.map((f) => f.name).join(', ')}.`,
          );
        }
        for (const file of result.files) {
          if (file.state === 'invalid') {
            this.problems.push(`${file.filename} could not be read: ${file.failure}`);
          } else if (file.state === 'duplicate') {
            this.problems.push(`${file.filename} is already in your vault.`);
          }
        }
      } catch (error) {
        const reason = error instanceof Error ? error.message : 'Something went wrong.';
        this.problems.push(`${current} was not uploaded: ${reason}`);
      } finally {
        this.sent = start + size;
        this.checking = false;
        if (!--this.#batches) this.total = this.sent = this.count = this.index = 0;
      }
    });
    return this.#queue;
  }
}

export const uploads = new Uploads();
