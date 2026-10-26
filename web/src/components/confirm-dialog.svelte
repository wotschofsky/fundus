<script module lang="ts">
  export type ConfirmRequest = {
    title: string;
    text: string;
    action: string;
    danger?: boolean;
    run: () => Promise<unknown>;
  };
</script>

<!-- Native modal <dialog>; the request's action runs only after confirmation. Set `request` to ask. -->
<script lang="ts">
  import ErrorNotice from './error-notice.svelte';

  let { request = $bindable() }: { request: ConfirmRequest | null } = $props();
  let dialog: HTMLDialogElement;
  let pending = $state(false);
  let error: unknown = $state(null);

  $effect(() => {
    if (request && !dialog.open) dialog.showModal();
    if (!request && dialog.open) dialog.close();
    error = null;
  });

  const close = () => {
    if (!pending) request = null;
  };

  async function confirm(event: SubmitEvent) {
    event.preventDefault();
    if (!request) return;
    pending = true;
    error = null;
    try {
      await request.run();
      request = null;
    } catch (e) {
      error = e;
    } finally {
      pending = false;
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="confirm"
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
>
  {#if request}
    <form method="dialog" onsubmit={confirm}>
      <h2>{request.title}</h2>
      <p>{request.text}</p>
      <ErrorNotice {error} />
      <div class="dialog-actions">
        <button type="button" onclick={close} disabled={pending}>Cancel</button>
        <button class={request.danger ? 'danger' : 'primary'} disabled={pending}>
          {pending ? 'Working…' : request.action}
        </button>
      </div>
    </form>
  {/if}
</dialog>
