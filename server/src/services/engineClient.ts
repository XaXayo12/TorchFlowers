import { env } from "../config/env.js";

export class EngineClientError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly body: unknown
  ) {
    super(message);
  }
}

export async function engineRequest<T>(
  path: string,
  init: RequestInit = {}
): Promise<T> {
  const response = await fetch(`${env.RUST_ENGINE_URL}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(init.headers ?? {})
    }
  });
  const text = await response.text();
  const body = text.length ? safeJson(text) : null;
  if (!response.ok) {
    throw new EngineClientError(
      typeof body === "object" && body && "error" in body
        ? String((body as { error: unknown }).error)
        : `Engine request failed with ${response.status}`,
      response.status,
      body
    );
  }
  return body as T;
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

