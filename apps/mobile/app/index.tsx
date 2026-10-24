import { useEffect, useState } from 'react';
import { Alert, AppState, PermissionsAndroid, Platform } from 'react-native';
import { Button, Column, Host, ScrollView, Text, TextInput } from '@expo/ui';
import * as DocumentPicker from 'expo-document-picker';
import { transfer, type QueueState } from '../modules/vault-transfer';

export default function UploadScreen() {
  const [queue, setQueue] = useState<QueueState>();
  const [settings, setSettings] = useState(false);
  const [server, setServer] = useState('');
  const [token, setToken] = useState('');
  const [timezone, setTimezone] = useState(Intl.DateTimeFormat().resolvedOptions().timeZone);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const update = async () => {
    try {
      setQueue(await transfer.status());
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => {
    void update();
    const interval = setInterval(() => {
      if (AppState.currentState === 'active') void update();
    }, 1000);
    const listener = AppState.addEventListener('change', (state) => {
      if (state === 'active') void update();
    });
    return () => {
      clearInterval(interval);
      listener.remove();
    };
  }, []);
  async function action(task: () => Promise<unknown>) {
    setBusy(true);
    setError('');
    try {
      await task();
      await update();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  async function pick() {
    if (Platform.OS === 'android' && Number(Platform.Version) >= 33)
      await PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.POST_NOTIFICATIONS);
    const result = await DocumentPicker.getDocumentAsync({
      type: ['audio/*', 'application/octet-stream'],
      multiple: true,
      copyToCacheDirectory: true,
    });
    if (!result.canceled)
      await transfer.enqueue(result.assets.map((file) => ({ uri: file.uri, name: file.name })));
  }
  const active = queue && ['uploading', 'copying', 'verifying'].includes(queue.state);
  return (
    <Host style={{ flex: 1, backgroundColor: '#ffffff' }} colorScheme="light" seedColor="#18181b">
      <ScrollView>
        <Column spacing={28} style={{ padding: 24 }}>
          <Column spacing={8}>
            <Text textStyle={{ fontSize: 28, fontWeight: '600' }}>
              {settings ? 'Connect your vault' : 'Recordings'}
            </Text>
            <Text>
              {settings
                ? 'Create an upload token in the web library’s Settings.'
                : 'Upload audio to your vault.'}
            </Text>
          </Column>
          {settings ? (
            <Column spacing={16}>
              <Text>Server address</Text>
              <TextInput
                defaultValue={server}
                onChangeText={setServer}
                placeholder="https://vault.example.com"
                keyboardType="url"
                autoCapitalize="none"
                autoCorrect={false}
              />
              <Text>Upload token</Text>
              <TextInput
                onChangeText={setToken}
                placeholder="Paste upload token"
                secureTextEntry
                autoCapitalize="none"
                autoCorrect={false}
              />
              <Text>Recorder timezone</Text>
              <TextInput
                defaultValue={timezone}
                onChangeText={setTimezone}
                placeholder="Europe/Berlin"
                autoCapitalize="none"
                autoCorrect={false}
              />
              <Text>
                The timezone should match the recorder’s clock. Your token is stored securely on
                this device.
              </Text>
              <Button
                label="Save connection"
                disabled={busy}
                onPress={() =>
                  void action(async () => {
                    await transfer.configure(server.trim(), token.trim(), timezone.trim());
                    setToken('');
                    setSettings(false);
                  })
                }
              />
              <Button label="Cancel" variant="text" onPress={() => setSettings(false)} />
            </Column>
          ) : (
            <>
              <Button
                label={queue?.server ? 'Connection settings' : 'Connect your vault'}
                variant="outlined"
                onPress={() => {
                  setServer(queue?.server ?? '');
                  setTimezone(queue?.timezone ?? timezone);
                  setSettings(true);
                }}
              />
              {queue?.files.length ? (
                <Column spacing={20}>
                  <Text textStyle={{ fontSize: 20, fontWeight: '600' }}>
                    {queue.state === 'verified'
                      ? 'Verified on your server'
                      : queue.state.charAt(0).toUpperCase() + queue.state.slice(1)}
                  </Text>
                  {queue.files.map((file) => (
                    <Column key={file.id} spacing={6}>
                      <Text numberOfLines={2} textStyle={{ fontWeight: '600' }}>
                        {file.name}
                      </Text>
                      <Text>
                        {file.state +
                          (file.length
                            ? ` · ${Math.round((file.offset / file.length) * 100)}% uploaded`
                            : '')}
                      </Text>
                    </Column>
                  ))}
                  {active && (
                    <Button
                      label="Pause"
                      variant="outlined"
                      onPress={() => void action(transfer.pause)}
                    />
                  )}
                  {['failed', 'paused', 'verifying'].includes(queue.state) && (
                    <Button label="Resume" onPress={() => void action(transfer.resume)} />
                  )}
                  {['failed', 'paused', 'verified'].includes(queue.state) && (
                    <Button
                      label="Clear local queue"
                      variant="text"
                      onPress={() =>
                        Alert.alert(
                          'Clear local copies?',
                          'Original recordings stay on your recorder. Any unverified staged copies will be removed from this app.',
                          [
                            { text: 'Cancel', style: 'cancel' },
                            {
                              text: 'Clear',
                              style: 'destructive',
                              onPress: () => void action(transfer.clear),
                            },
                          ],
                        )
                      }
                    />
                  )}
                </Column>
              ) : (
                <Column spacing={12}>
                  <Text textStyle={{ fontSize: 22, fontWeight: '600' }}>
                    Add your first recording
                  </Text>
                  <Text>Choose audio from a connected recorder or your files.</Text>
                  <Text>WAV files become verified, lossless FLAC when possible.</Text>
                </Column>
              )}
              <Button
                label="Add recordings"
                disabled={busy || !queue?.server || !['idle', 'verified'].includes(queue.state)}
                onPress={() => void action(pick)}
              />
              <Text>
                Originals stay untouched. Keep the app open during preparation. Uploads can continue
                in the background.
              </Text>
            </>
          )}
          {error || queue?.error ? (
            <Text textStyle={{ color: '#be3540' }}>{error || queue?.error || ''}</Text>
          ) : null}
        </Column>
      </ScrollView>
    </Host>
  );
}
