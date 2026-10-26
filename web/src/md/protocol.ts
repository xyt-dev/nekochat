// Messages between the main thread and the markdown worker.

export interface MdRequest {
  /** Message identity (node id); the worker remembers what it last sent per key. */
  key: string;
  text: string;
  /** `false` while streaming: the last block is rendered without math and highlighting. */
  final: boolean;
  /** Blocks the main thread already holds from the previous response for this key. */
  base: number;
  /** Forget the key. */
  drop?: boolean;
}

export interface MdResponse {
  key: string;
  /** Block count of the whole message. */
  total: number;
  /** Index of `html[0]`; blocks before it are unchanged. */
  start: number;
  html: string[];
  dropped?: boolean;
}
