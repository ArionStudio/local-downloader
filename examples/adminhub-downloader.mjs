import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'

/** Run the local CLI without a shell; each event is one JSON line. */
export function runDownloader(args, { executable = 'downloader-cli', request, onEvent, signal } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { stdio: ['pipe', 'pipe', 'pipe'] })
    let final
    let stderr = ''
    let protocolError
    child.stderr.setEncoding('utf8')
    child.stderr.on('data', text => { stderr = (stderr + text).slice(-16000) })
    const lines = createInterface({ input: child.stdout })
    lines.on('line', line => {
      try {
        const event = JSON.parse(line)
        if (event.type === 'result') final = event
        else onEvent?.(event)
      } catch (error) {
        protocolError = error
        child.kill('SIGINT')
      }
    })
    const cancel = () => child.kill('SIGINT')
    signal?.addEventListener('abort', cancel, { once: true })
    child.on('error', reject)
    child.on('close', code => {
      signal?.removeEventListener('abort', cancel)
      if (protocolError) return reject(protocolError)
      if (code !== 0 || !final?.ok) {
        const error = new Error(final?.error?.message || final?.result?.errors?.[0]?.message || final?.result?.jobs?.find(job => job.errorMessage)?.errorMessage || stderr || `Downloader exited with ${code}`)
        error.result = final?.result // Includes finished files after cancellation.
        error.exitCode = code
        return reject(error)
      }
      resolve(final.result)
    })
    child.stdin.on('error', () => {}) // Early argument errors may close stdin.
    child.stdin.end(request === undefined ? undefined : JSON.stringify(request))
    if (signal?.aborted) cancel()
  })
}

/** Prepare selected video sources locally. Upload only the returned files. */
export async function downloadForAdminHub(request, options = {}) {
  const original = await runDownloader(['download', '--request', '-', '--events'], {
    ...options,
    request: { ...request, outputProfile: 'original' },
  })
  if (!original.files.length) throw new Error('No finished videos were returned for preparation.')
  const files = []
  for (const file of original.files) {
    if (file.missing) throw new Error(`Downloaded file is missing: ${file.path}`)
    const prepared = await runDownloader(['prepare', file.path, '--events'], options)
    files.push(prepared.file)
  }
  return { jobs: original.jobs, originals: original.files, files }
}
