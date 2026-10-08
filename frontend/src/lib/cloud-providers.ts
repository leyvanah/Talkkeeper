/**
 * Which summary providers send the meeting text off this computer.
 *
 * Four are cloud services by definition. The built-in model never leaves the
 * machine. Ollama and a custom OpenAI-compatible server go wherever their
 * address points: on this computer by default, but an address can name another
 * machine, and then the text goes there.
 *
 * "On this computer" is decided the way the backend's local-only mode decides
 * it (`network_policy::is_loopback_url`), so the window never promises what the
 * backend would refuse or allow differently.
 */

export const CLOUD_SUMMARY_PROVIDERS = ['claude', 'groq', 'openai', 'openrouter'] as const
export const LOCAL_SUMMARY_PROVIDERS = ['builtin-ai', 'ollama', 'custom-openai'] as const

export type CloudSummaryProvider = (typeof CLOUD_SUMMARY_PROVIDERS)[number]
export type LocalSummaryProvider = (typeof LOCAL_SUMMARY_PROVIDERS)[number]
export type SummaryProvider = CloudSummaryProvider | LocalSummaryProvider

/** Ollama's own default, used when no address has been entered. */
export const DEFAULT_OLLAMA_ENDPOINT = 'http://localhost:11434'

export function isCloudProvider(provider: string): provider is CloudSummaryProvider {
  return (CLOUD_SUMMARY_PROVIDERS as readonly string[]).includes(provider)
}

/** Whether `url` points at this computer: `localhost`, `127.x.x.x` or `::1`. */
export function isLoopbackUrl(url: string): boolean {
  let host: string
  try {
    host = new URL(url.trim()).hostname.toLowerCase()
  } catch {
    return false
  }
  if (host === 'localhost') return true
  if (host === '[::1]') return true
  // WHATWG parsing has already turned every IPv4 spelling into a dotted quad.
  const quad = host.match(/^(\d+)\.\d+\.\d+\.\d+$/)
  return quad !== null && quad[1] === '127'
}

/**
 * Whether choosing `provider` with this address sends the meeting text to
 * another machine. An address not typed in yet is not counted: there is
 * nowhere for the text to go until there is one.
 */
export function sendsOffThisComputer(provider: string, endpoint?: string | null): boolean {
  if (isCloudProvider(provider)) return true
  const address = endpoint?.trim()
  switch (provider) {
    case 'ollama':
      return !isLoopbackUrl(address || DEFAULT_OLLAMA_ENDPOINT)
    case 'custom-openai':
      return Boolean(address) && !isLoopbackUrl(address as string)
    default:
      return false
  }
}
