import {
  Client,
  Events,
  GatewayIntentBits,
  PermissionFlagsBits,
  type ChatInputCommandInteraction,
  type InteractionReplyOptions
} from "discord.js";
import type {
  Account,
  Bot,
  CapabilityStatus,
  DeviceAuthSession,
  LogEntry,
  Server
} from "@rustrock/shared";
import { adminUserIds, allowedRoleIds, env } from "../config/env.js";
import { EngineClientError, engineRequest } from "../services/engineClient.js";
import { registerCommands, RUSTROCK_COMMAND } from "./commands.js";
import {
  accountsEmbed,
  authSessionEmbed,
  botsEmbed,
  errorEmbed,
  logsEmbed,
  serversEmbed,
  statusEmbed,
  validationEmbed
} from "./format.js";

interface ImportAccountResponse {
  session: DeviceAuthSession;
}

interface PollAuthResponse {
  status: string;
  account: Account | null;
}

export async function startDiscordBot() {
  const registrationScope = await registerCommands();
  const client = new Client({ intents: [GatewayIntentBits.Guilds] });

  client.once(Events.ClientReady, (readyClient) => {
    console.log(
      `RustRock Discord bot logged in as ${readyClient.user.tag}; registered ${registrationScope} commands`
    );
  });

  client.on(Events.InteractionCreate, async (interaction) => {
    if (!interaction.isChatInputCommand() || interaction.commandName !== RUSTROCK_COMMAND) {
      return;
    }
    if (!isAuthorized(interaction)) {
      await interaction.reply(ephemeral({ content: "You are not authorized to manage RustRock." }));
      return;
    }
    await handleRustRockCommand(interaction);
  });

  await client.login(env.DISCORD_BOT_TOKEN);
}

async function handleRustRockCommand(interaction: ChatInputCommandInteraction) {
  await interaction.deferReply({ ephemeral: true });
  try {
    const subcommand = interaction.options.getSubcommand(true);
    switch (subcommand) {
      case "status":
        await showStatus(interaction);
        return;
      case "accounts":
        await interaction.editReply({ embeds: [accountsEmbed(await listAccounts())] });
        return;
      case "import":
        await importAccount(interaction);
        return;
      case "poll-auth":
        await pollAuth(interaction);
        return;
      case "provision":
        await provisionAccount(interaction);
        return;
      case "add-server":
        await addServer(interaction);
        return;
      case "servers":
        await interaction.editReply({ embeds: [serversEmbed(await listServers())] });
        return;
      case "create-bot":
        await createBot(interaction);
        return;
      case "bots":
        await interaction.editReply({ embeds: [botsEmbed(await listBots())] });
        return;
      case "start-bot":
        await botAction(interaction, "start");
        return;
      case "stop-bot":
        await botAction(interaction, "stop");
        return;
      case "validate":
        await validateServer(interaction);
        return;
      case "logs":
        await showLogs(interaction);
        return;
      default:
        await interaction.editReply(`Unknown RustRock subcommand: ${subcommand}`);
    }
  } catch (error) {
    await interaction.editReply({ embeds: [errorEmbed(formatEngineError(error))] });
  }
}

async function showStatus(interaction: ChatInputCommandInteraction) {
  const [accounts, servers, bots, logs] = await Promise.all([
    listAccounts(),
    listServers(),
    listBots(),
    listLogs(20)
  ]);
  await interaction.editReply({ embeds: [statusEmbed(accounts, servers, bots, logs)] });
}

async function importAccount(interaction: ChatInputCommandInteraction) {
  const email = interaction.options.getString("email", true);
  const response = await engineRequest<ImportAccountResponse>("/api/accounts", {
    method: "POST",
    body: JSON.stringify({ email })
  });
  await interaction.editReply({ embeds: [authSessionEmbed(response.session)] });
}

async function pollAuth(interaction: ChatInputCommandInteraction) {
  const sessionId = interaction.options.getString("session-id", true);
  const response = await engineRequest<PollAuthResponse>(`/api/auth/sessions/${sessionId}/poll`, {
    method: "POST",
    body: "{}"
  });
  if (response.account) {
    await interaction.editReply({
      content:
        "Authentication finished. Provisioning ran through Xbox, XSTS, PlayFab, entitlement session/start, legacy Bedrock auth, and JWT-chain generation.",
      embeds: [accountsEmbed([response.account])]
    });
    return;
  }
  await interaction.editReply(`Authentication is still ${response.status}. Try again after the poll interval.`);
}

async function provisionAccount(interaction: ChatInputCommandInteraction) {
  const accountId = interaction.options.getString("account-id", true);
  const account = await engineRequest<Account>(`/api/accounts/${accountId}/provision`, {
    method: "POST",
    body: "{}"
  });
  await interaction.editReply({ embeds: [accountsEmbed([account])] });
}

async function addServer(interaction: ChatInputCommandInteraction) {
  const name = interaction.options.getString("name", true);
  const host = interaction.options.getString("host", true);
  const port = interaction.options.getInteger("port") ?? 19132;
  const server = await engineRequest<Server>("/api/servers", {
    method: "POST",
    body: JSON.stringify({ name, host, port })
  });
  await interaction.editReply({ content: "Server added.", embeds: [serversEmbed([server])] });
}

async function createBot(interaction: ChatInputCommandInteraction) {
  const accountId = interaction.options.getString("account-id", true);
  const serverId = interaction.options.getString("server-id", true);
  const bot = await engineRequest<Bot>("/api/bots", {
    method: "POST",
    body: JSON.stringify({ account_id: accountId, server_id: serverId })
  });
  await interaction.editReply({ content: "Bot created.", embeds: [botsEmbed([bot])] });
}

async function botAction(interaction: ChatInputCommandInteraction, action: "start" | "stop") {
  const botId = interaction.options.getString("bot-id", true);
  await engineRequest(`/api/bots/${botId}/${action}`, { method: "POST", body: "{}" });
  await interaction.editReply(`Bot ${action === "start" ? "start requested" : "stopped"}: \`${botId}\``);
}

async function validateServer(interaction: ChatInputCommandInteraction) {
  const accountId = interaction.options.getString("account-id", true);
  const host = interaction.options.getString("host", true);
  const port = interaction.options.getInteger("port") ?? 19132;
  const status = await engineRequest<CapabilityStatus>("/api/validate-real-server", {
    method: "POST",
    body: JSON.stringify({ account_id: accountId, host, port })
  });
  await interaction.editReply({ embeds: [validationEmbed(status)] });
}

async function showLogs(interaction: ChatInputCommandInteraction) {
  const limit = interaction.options.getInteger("limit") ?? 10;
  await interaction.editReply({ embeds: [logsEmbed(await listLogs(limit))] });
}

async function listAccounts() {
  return engineRequest<Account[]>("/api/accounts");
}

async function listServers() {
  return engineRequest<Server[]>("/api/servers");
}

async function listBots() {
  return engineRequest<Bot[]>("/api/bots");
}

async function listLogs(limit: number) {
  return engineRequest<LogEntry[]>(`/api/logs?limit=${limit}`);
}

function isAuthorized(interaction: ChatInputCommandInteraction) {
  if (adminUserIds.has(interaction.user.id)) {
    return true;
  }
  const roleIds = interactionRoleIds(interaction);
  if (allowedRoleIds.size > 0 && roleIds.some((roleId) => allowedRoleIds.has(roleId))) {
    return true;
  }
  if (allowedRoleIds.size === 0 && adminUserIds.size === 0) {
    return Boolean(interaction.memberPermissions?.has(PermissionFlagsBits.Administrator));
  }
  return false;
}

function interactionRoleIds(interaction: ChatInputCommandInteraction) {
  const roles = interaction.member && "roles" in interaction.member ? interaction.member.roles : [];
  if (Array.isArray(roles)) {
    return roles;
  }
  if (roles && typeof roles === "object" && "cache" in roles) {
    return Array.from(roles.cache.keys());
  }
  return [];
}

function ephemeral(options: InteractionReplyOptions): InteractionReplyOptions {
  return { ...options, ephemeral: true };
}

function formatEngineError(error: unknown) {
  if (error instanceof EngineClientError) {
    return new Error(`${error.message} (${error.status})`);
  }
  return error;
}
