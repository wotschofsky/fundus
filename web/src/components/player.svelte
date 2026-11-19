<!-- A recording's player, on its own page or a share link's: the picture once the video is ready,
     else the audio. The transcript comes from the audio either way. -->
<script lang="ts">
  import type { Entry } from '../api';
  import type { Playback } from '../playback.svelte';

  let {
    playback,
    base,
    audio,
    video,
    busy = false,
    onretry,
  }: {
    playback: Playback;
    /** Where `/audio` and `/video` are served: the owner's API or a share link. */
    base: string;
    audio: boolean;
    video: (Omit<NonNullable<Entry['video']>, 'failure'> & { failure?: string | null }) | null;
    busy?: boolean;
    /** Prepares a failed video again (the owner's page only). */
    onretry?: () => void;
  } = $props();
  /** The video failed to load (a format this browser lacks, or a dropped connection), so the
   * page plays the audio if it has it. */
  let unplayable = $state(false);
  const picture = $derived(video?.state === 'ready' && !unplayable ? video : null);
  const playable = $derived(!!picture || audio);
</script>

{#if audio || video}
  <div class="player">
    <!-- A video is sized before it loads and is never taller than 40% of the screen. -->
    {#if playable}
      <svelte:element
        this={picture ? 'video' : 'audio'}
        bind:this={playback.media}
        ontimeupdate={playback.timeUpdate}
        onplay={playback.played}
        onpause={playback.paused}
        onloadedmetadata={playback.loaded}
        onerror={() => picture && (unplayable = true)}
        controls
        playsinline
        preload="metadata"
        style:aspect-ratio={picture && `${picture.width} / ${picture.height}`}
        style:width={picture && `min(100%, calc(40vh * ${picture.width} / ${picture.height}))`}
        src={`${base}/${picture ? 'video' : 'audio'}`}
      ></svelte:element>
    {/if}
    <div class="player-row">
      <p class={['player-note', video?.state === 'failed' && 'error']} role="status">
        {#if video?.state === 'preparing'}
          Preparing the video.{audio ? ' You can listen in the meantime.' : ''}
        {:else if video?.state === 'failed'}
          The video could not be prepared{video.failure ? `: ${video.failure}` : '.'}
          {#if onretry}
            <button class="text-button" aria-label="Retry video" disabled={busy} onclick={onretry}
              >Retry</button
            >
          {/if}
        {:else if unplayable}
          The video could not be played{audio ? ', so you hear the audio.' : '.'}
          <!-- A dropped connection fails the same way as a format this browser lacks. -->
          <button
            class="text-button"
            aria-label="Try again to play the video"
            onclick={() => (unplayable = false)}>Try again</button
          >
        {/if}
      </p>
      {#if playable}
        <label>
          <input type="checkbox" bind:checked={playback.follow} />
          Follow transcript
        </label>
      {/if}
    </div>
  </div>
{/if}
