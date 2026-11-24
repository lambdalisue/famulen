/// <reference types="@solidjs/start/env" />

declare namespace App {
  interface RequestEventLocals {
    /** The secretary's portrait, while the backend answers. */
    portrait?: import("./lib/portrait").Portrait;
  }
}
