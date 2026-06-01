import { EmbedBuilder } from "discord.js";
import type { Account, Bot, CapabilityStatus, LogEntry, Server } from "@rustrock/shared";

export const OK_COLOR = 0x58725d;
export const WARN_COLOR = 0x9f7a2f;
export const ERROR_COLOR = 0x9b3a34;
export const INFO_COLOR = 0x2f7485;

export function statusEmbed(accounts: Account[], servers: Server[], bots: Bot[], logs: LogEntry[]) {
  return new EmbedBuilder()
    .setTitle("RustRock Status")
    .setColor(INFO_COLOR)
    .addFields(
      { name: "Accounts", value: String(accounts.length), inline: true },
      { name: "Servers", value: String(servers.length), inline: true },
      { name: "Bots", value: String(bots.length), inline: true },
      { name: "Recent Diagnostics", value: String(logs.length), inline: true }
    );
}

export function accountsEmbed(accounts: Account[]) {
  const embed = new EmbedBuilder().setTitle("Accounts").setColor(INFO_COLOR);
  if (!accounts.length) {
    return embed.setDescription("No accounts imported yet.");
  }
  return embed.setDescription(
    truncate(
      accounts
        .map((account) =>
          [
            `**${escapeMarkdown(account.email)}**`,
            `id: \`${account.id}\``,
            `ms=${account.microsoft_status}`,
            `xbox=${account.xbox_status}`,
            `xsts=${account.xsts_status}`,
            `playfab=${account.playfab_status}`,
            `entitlement=${account.entitlement_status}`,
            `bedrock=${account.bedrock_auth_status}`,
            account.last_error ? `last_error=${account.last_error}` : null
          ]
            .filter(Boolean)
            .join(" | ")
        )
        .join("\n\n"),
      3900
    )
  );
}

export function authSessionEmbed(session: {
  id: string;
  user_code: string;
  verification_uri: string;
  expires_at: string;
  interval_seconds: number;
}) {
  return new EmbedBuilder()
    .setTitle("Microsoft Device Auth Started")
    .setColor(WARN_COLOR)
    .setDescription("Open the verification URL, enter the code, then run `/rustrock poll-auth`.")
    .addFields(
      { name: "User Code", value: `\`${session.user_code}\``, inline: true },
      { name: "Session Id", value: `\`${session.id}\`` },
      { name: "Verification URL", value: session.verification_uri },
      { name: "Poll Interval", value: `${session.interval_seconds}s`, inline: true },
      { name: "Expires At", value: session.expires_at, inline: true }
    );
}

export function serversEmbed(servers: Server[]) {
  const embed = new EmbedBuilder().setTitle("Servers").setColor(INFO_COLOR);
  if (!servers.length) {
    return embed.setDescription("No servers configured yet.");
  }
  return embed.setDescription(
    truncate(
      servers
        .map(
          (server) =>
            `**${escapeMarkdown(server.name)}** ${server.host}:${server.port} | id: \`${server.id}\` | protocol=${server.protocol_version}`
        )
        .join("\n"),
      3900
    )
  );
}

export function botsEmbed(bots: Bot[]) {
  const embed = new EmbedBuilder().setTitle("Bots").setColor(INFO_COLOR);
  if (!bots.length) {
    return embed.setDescription("No bots created yet.");
  }
  return embed.setDescription(
    truncate(
      bots
        .map((bot) => {
          const capabilities = capabilitySummary(bot.capabilities_json as CapabilityStatus);
          return [
            `**${bot.status}** | id: \`${bot.id}\``,
            `account=\`${bot.account_id}\``,
            `server=\`${bot.server_id}\``,
            capabilities ? `capabilities=${capabilities}` : null,
            bot.last_error ? `last_error=${bot.last_error}` : null
          ]
            .filter(Boolean)
            .join(" | ");
        })
        .join("\n\n"),
      3900
    )
  );
}

export function validationEmbed(status: CapabilityStatus) {
  return new EmbedBuilder()
    .setTitle("Real Server Validation")
    .setColor(status.missing_capabilities.length ? WARN_COLOR : OK_COLOR)
    .setDescription(capabilityLines(status).join("\n"))
    .addFields({
      name: "Missing",
      value: status.missing_capabilities.length
        ? status.missing_capabilities.map((item) => `\`${item}\``).join(", ")
        : "None"
    });
}

export function logsEmbed(logs: LogEntry[]) {
  const embed = new EmbedBuilder().setTitle("Diagnostics").setColor(INFO_COLOR);
  if (!logs.length) {
    return embed.setDescription("No diagnostics stored yet.");
  }
  return embed.setDescription(
    truncate(
      logs
        .map((log) =>
          [
            `\`${log.created_at}\``,
            `**${log.level}/${log.category}**`,
            log.step ? `step=${log.step}` : null,
            log.status_code ? `status=${log.status_code}` : null,
            truncate(log.message, 180)
          ]
            .filter(Boolean)
            .join(" | ")
        )
        .join("\n"),
      3900
    )
  );
}

export function errorEmbed(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return new EmbedBuilder()
    .setTitle("RustRock Command Failed")
    .setColor(ERROR_COLOR)
    .setDescription(truncate(message, 3900));
}

function capabilitySummary(status: CapabilityStatus | Record<string, unknown>): string {
  const typed = status as CapabilityStatus;
  if (!Array.isArray(typed.missing_capabilities)) {
    return "";
  }
  return typed.missing_capabilities.length
    ? `missing ${typed.missing_capabilities.join(", ")}`
    : "all validation probes passed";
}

function capabilityLines(status: CapabilityStatus) {
  return [
    ["Login", status.login],
    ["Spawn", status.spawn],
    ["Keepalive", status.keepalive],
    ["Chat", status.chat],
    ["Forms", status.forms],
    ["Inventory transactions", status.inventory_transactions],
    ["Movement", status.movement],
    ["Disconnect handling", status.disconnect_handling]
  ].map(([name, ok]) => `${ok ? "OK" : "MISSING"} ${name}`);
}

export function truncate(value: string, maxLength: number) {
  if (value.length <= maxLength) {
    return value;
  }
  return `${value.slice(0, Math.max(0, maxLength - 3))}...`;
}

function escapeMarkdown(value: string) {
  return value.replace(/([\\_*~`>|])/g, "\\$1");
}
