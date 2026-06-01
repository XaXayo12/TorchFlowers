import dotenv from "dotenv";
import { z } from "zod";

dotenv.config();

const EnvSchema = z.object({
  RUST_ENGINE_URL: z.string().url().default("http://127.0.0.1:9080"),
  DISCORD_BOT_TOKEN: z.string().min(1),
  DISCORD_CLIENT_ID: z.string().min(1),
  DISCORD_GUILD_ID: z.string().optional(),
  DISCORD_ALLOWED_ROLE_IDS: z.string().default(""),
  DISCORD_ADMIN_USER_IDS: z.string().default("")
});

export const env = EnvSchema.parse(process.env);

export const allowedRoleIds = csvSet(env.DISCORD_ALLOWED_ROLE_IDS);
export const adminUserIds = csvSet(env.DISCORD_ADMIN_USER_IDS);

function csvSet(value: string): Set<string> {
  return new Set(
    value
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean)
  );
}
