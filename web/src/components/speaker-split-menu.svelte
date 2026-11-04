<script lang="ts">
  import { Ellipsis, UsersRound, UserRoundX } from '@lucide/svelte';

  let {
    label,
    onsplit,
    class: className = '',
    notSpeaker,
  }: {
    label: string;
    notSpeaker?: string;
    onsplit: () => void;
    class?: string;
  } = $props();
  const id = $props.id();
  let menu = $state<HTMLDivElement>();
  let trigger = $state<HTMLButtonElement>();
  let menuOpen = $state(false);
  let top = $state(0);
  let left = $state(0);
  $effect(() => {
    if (!menuOpen) return;
    const close = () => menu?.hidePopover();
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
    };
  });
</script>

<button
  bind:this={trigger}
  class={['icon-button speaker-split-trigger', className]}
  aria-label={label}
  title={label}
  popovertarget={id}
>
  <Ellipsis size={16} aria-hidden="true" />
</button>
<div
  bind:this={menu}
  {id}
  class="turn-menu"
  popover="auto"
  style:top={`${top}px`}
  style:left={`${left}px`}
  onbeforetoggle={(event) => {
    if (event.newState !== 'open') return;
    const rect = trigger?.getBoundingClientRect();
    if (!rect) return;
    top = Math.min(rect.bottom + 4, window.innerHeight - 52);
    left = Math.max(8, Math.min(rect.right - 176, window.innerWidth - 184));
  }}
  ontoggle={(event) => (menuOpen = event.newState === 'open')}
>
  <button
    onclick={() => {
      menu?.hidePopover();
      onsplit();
    }}
  >
    {#if notSpeaker}<UserRoundX size={14} aria-hidden="true" />{:else}<UsersRound
        size={14}
        aria-hidden="true"
      />{/if}
    <span>{notSpeaker ? `Not ${notSpeaker}` : 'Multiple speakers'}</span>
  </button>
</div>
