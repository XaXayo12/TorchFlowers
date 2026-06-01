import {
  REST,
  Routes,
  SlashCommandBuilder,
  type RESTPostAPIApplicationCommandsJSONBody
} from "discord.js";
import { env } from "../config/env.js";

export const RUSTROCK_COMMAND = "rustrock";

export function buildCommands(): RESTPostAPIApplicationCommandsJSONBody[] {
  return [
    new SlashCommandBuilder()
      .setName(RUSTROCK_COMMAND)
      .setDescription("Manage RustRock Bedrock bots")
      .addSubcommand((command) =>
        command.setName("status").setDescription("Show account, server, bot, and diagnostics counts")
      )
      .addSubcommand((command) =>
        command.setName("accounts").setDescription("List Microsoft/Xbox/PlayFab/entitlement status")
      )
      .addSubcommand((command) =>
        command
          .setName("import")
          .setDescription("Import a Microsoft account and start device-code auth")
          .addStringOption((option) =>
            option
              .setName("email")
              .setDescription("Microsoft account email address")
              .setRequired(true)
          )
      )
      .addSubcommand((command) =>
        command
          .setName("poll-auth")
          .setDescription("Poll a Microsoft device-code auth session")
          .addStringOption((option) =>
            option
              .setName("session-id")
              .setDescription("Auth session id returned by /rustrock import")
              .setRequired(true)
          )
      )
      .addSubcommand((command) =>
        command
          .setName("provision")
          .setDescription("Run Xbox, XSTS, PlayFab, entitlement, and Bedrock auth for an account")
          .addStringOption((option) =>
            option
              .setName("account-id")
              .setDescription("Account id")
              .setRequired(true)
          )
      )
      .addSubcommand((command) =>
        command
          .setName("add-server")
          .setDescription("Add a Bedrock server target")
          .addStringOption((option) =>
            option.setName("name").setDescription("Display name").setRequired(true)
          )
          .addStringOption((option) =>
            option.setName("host").setDescription("Server host").setRequired(true)
          )
          .addIntegerOption((option) =>
            option
              .setName("port")
              .setDescription("Server port")
              .setMinValue(1)
              .setMaxValue(65535)
              .setRequired(false)
          )
      )
      .addSubcommand((command) =>
        command.setName("servers").setDescription("List configured Bedrock servers")
      )
      .addSubcommand((command) =>
        command
          .setName("create-bot")
          .setDescription("Create a bot from an account and server")
          .addStringOption((option) =>
            option.setName("account-id").setDescription("Account id").setRequired(true)
          )
          .addStringOption((option) =>
            option.setName("server-id").setDescription("Server id").setRequired(true)
          )
      )
      .addSubcommand((command) =>
        command.setName("bots").setDescription("List bots and capability probes")
      )
      .addSubcommand((command) =>
        command
          .setName("start-bot")
          .setDescription("Start a bot session")
          .addStringOption((option) =>
            option.setName("bot-id").setDescription("Bot id").setRequired(true)
          )
      )
      .addSubcommand((command) =>
        command
          .setName("stop-bot")
          .setDescription("Stop a bot session")
          .addStringOption((option) =>
            option.setName("bot-id").setDescription("Bot id").setRequired(true)
          )
      )
      .addSubcommand((command) =>
        command
          .setName("validate")
          .setDescription("Run the real-server validation probe")
          .addStringOption((option) =>
            option.setName("account-id").setDescription("Account id").setRequired(true)
          )
          .addStringOption((option) =>
            option.setName("host").setDescription("Server host").setRequired(true)
          )
          .addIntegerOption((option) =>
            option
              .setName("port")
              .setDescription("Server port")
              .setMinValue(1)
              .setMaxValue(65535)
              .setRequired(false)
          )
      )
      .addSubcommand((command) =>
        command
          .setName("logs")
          .setDescription("Show recent authentication and bot diagnostics")
          .addIntegerOption((option) =>
            option
              .setName("limit")
              .setDescription("Number of log rows")
              .setMinValue(1)
              .setMaxValue(20)
              .setRequired(false)
          )
      )
      .toJSON()
  ];
}

export async function registerCommands() {
  const rest = new REST({ version: "10" }).setToken(env.DISCORD_BOT_TOKEN);
  const commands = buildCommands();
  if (env.DISCORD_GUILD_ID) {
    await rest.put(
      Routes.applicationGuildCommands(env.DISCORD_CLIENT_ID, env.DISCORD_GUILD_ID),
      { body: commands }
    );
    return "guild";
  }
  await rest.put(Routes.applicationCommands(env.DISCORD_CLIENT_ID), { body: commands });
  return "global";
}
