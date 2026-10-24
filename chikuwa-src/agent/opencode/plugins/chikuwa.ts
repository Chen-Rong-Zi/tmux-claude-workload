/**
 * Chikuwa plugin for OpenCode
 *
 * Integrates OpenCode with chikuwa TUI for agent state tracking.
 *
 * Consensus enforcement: ALL state data passes through typed emitter functions
 * (one per event type), which are the ONLY callers of `sendStateDebounced`.
 * Emitter functions construct the full `ConsensusStateData` internally —
 * callers only pass typed primitive parameters.
 *
 * Type definitions are auto-generated from Rust structs via build.rs.
 */

import { appendFileSync, mkdirSync, readdirSync } from "fs";
import { join } from "path";
import { createConnection } from "net";

import type { AgentState, ActiveTool } from "./opencode-types";

// ─── Consensus types — must match Rust's OpenCodeState ─────────────────

/** The exact data shape expected by the Rust side for OpenCode agents. */
interface ConsensusStateData {
	session_id?: string;
	status: "started" | "running" | "waiting" | "permission" | "ended";
	event_type: string;
	event_emoji?: string;
	tool_name?: string;
	tool_detail?: string;
	active_tools: ActiveTool[];
	is_busy: boolean;
}

// ─── Logging ────────────────────────────────────────────────────────────

const LOG_FILE = "/tmp/chikuwa-opencode-plugin.log";
const RAW_LOG_FILE = "/tmp/chikuwa.raw.log";
const CHIKUWA_STATE_DIR = process.env.XDG_RUNTIME_DIR
	? join(process.env.XDG_RUNTIME_DIR, "chikuwa")
	: "/tmp/chikuwa";
const AGENT_STATES_FILE = join(CHIKUWA_STATE_DIR, "agent_states.jsonl");

const log = (msg: string, data?: unknown) => {
	const timestamp = new Date().toISOString();
	const logMsg = `[${timestamp}] ${msg}${data ? " " + JSON.stringify(data) : ""}\n`;
	try {
		appendFileSync(LOG_FILE, logMsg);
	} catch {
		// ignore
	}
};

const logRaw = (data: unknown) => {
	try {
		appendFileSync(RAW_LOG_FILE, JSON.stringify(data) + "\n", "utf8");
	} catch {
		// ignore
	}
};

// ─── Time helpers ───────────────────────────────────────────────────────

const now = () => Math.floor(Date.now() / 1000);

// ─── Consensus enforcement — single chokepoint for all output ───────────

/**
 * Wrap raw OpenCode state data into a consensus-compliant AgentState.
 * This is the ONLY place where an AgentState with `type: "opencode"`
 * is constructed.
 */
function enforceConsensus(tmuxPane: string, data: ConsensusStateData): AgentState {
	return {
		tmux_pane: tmuxPane,
		updated_at: now(),
		data: {
			...data,
			type: "opencode",
		},
	};
}

// ─── IPC & persistence ──────────────────────────────────────────────────

const appendAgentState = (state: AgentState) => {
	const json = JSON.stringify(state);
	if (!json.includes('"type":"opencode"')) {
		log("CRITICAL: state missing type discriminator, not writing to JSONL");
		return;
	}
	try {
		mkdirSync(CHIKUWA_STATE_DIR, { recursive: true });
		appendFileSync(AGENT_STATES_FILE, json + "\n", "utf8");
	} catch (e) {
		log("Failed to append agent state", { error: String(e) });
	}
};

const sendToIpc = async (state: AgentState) => {
	const json = JSON.stringify(state) + "\n";
	if (!json.includes('"type":"opencode"')) {
		log("CRITICAL: state missing type discriminator, not sending to IPC");
		return;
	}
	try {
		const files = readdirSync(CHIKUWA_STATE_DIR);
		const socketFiles = files.filter(f => f.endsWith(".sock"));
		if (socketFiles.length === 0) {
			log("No socket file found");
			return;
		}
		log("Sending IPC to sockets", { count: socketFiles.length, files: socketFiles, jsonLen: json.length, jsonPreview: json.substring(0, 300) });
		for (const socketFile of socketFiles) {
			const socketPath = join(CHIKUWA_STATE_DIR, socketFile);
			const client = createConnection(socketPath, () => {
				log("IPC raw json length", { socket: socketFile, len: json.length, full: json.substring(0, 500) });
				client.write(json, () => {
					client.end(() => {
						log("IPC wrote to", { socket: socketFile, tool: state.data.tool_name, status: state.data.status });
					});
				});
			});
			client.on("error", (err: any) => {
				log("IPC connection error", { socket: socketFile, error: String(err) });
			});
		}
	} catch (e) {
		log("Failed to send IPC", { error: String(e) });
	}
};

// ─── Tool detail extraction ─────────────────────────────────────────────

const extractToolDetail = (tool: string, args: Record<string, unknown>): string | undefined => {
	switch (tool) {
		case "bash": return args.command as string | undefined;
		case "read": {
			const path = args.filePath as string | undefined;
			const offset = args.offset as number | undefined;
			return offset ? `${path}:${offset}` : path;
		}
		case "write":
		case "edit": return args.filePath as string | undefined;
		case "grep":
		case "glob": return args.pattern as string | undefined;
		case "web_fetch": return args.url as string | undefined;
		case "web_search": return args.query as string | undefined;
		case "task": return args.description as string | undefined;
		default: return undefined;
	}
};

// ─── OpenCode plugin entry point ────────────────────────────────────────

export const ChikuwaPlugin = async ({ client, directory, $ }: { client: any; directory: string; $: any }) => {
	const tmuxPane = process.env.TMUX_PANE || "";
	if (!tmuxPane) {
		log("Not in tmux, plugin disabled");
		return {};
	}

	log("Plugin loading", { tmuxPane, directory });

	let currentSessionId = "";
	let activeTools: ActiveTool[] = [];
	let isBusy = false;

	const CRITICAL_EVENTS = new Set([
		"session.created",
		"session.deleted",
		"session.status",
		"session.idle",
		"permission.asked",
		"permission.replied",
		"command.executed",
	]);

	let debounceTimer: ReturnType<typeof setTimeout> | null = null;
	let pendingState: ConsensusStateData | null = null;

	const sendState = async (data: ConsensusStateData) => {
		const state = enforceConsensus(tmuxPane, data);
		appendAgentState(state);
		await sendToIpc(state);
	};

	const sendStateDebounced = async (data: ConsensusStateData, eventType?: string) => {
		if (eventType && CRITICAL_EVENTS.has(eventType)) {
			await sendState(data);
			return;
		}
		pendingState = data;
		if (debounceTimer) return;
		debounceTimer = setTimeout(async () => {
			debounceTimer = null;
			if (pendingState) {
				await sendState(pendingState);
				pendingState = null;
			}
		}, 100);
	};

	const updateSession = (sessionId: string) => {
		if (sessionId && sessionId !== currentSessionId) {
			currentSessionId = sessionId;
			log("Session updated", { sessionId });
		}
	};

	// ── Typed emitter functions ───────────────────────────────────────
	//
	// Each function constructs the correct ConsensusStateData internally.
	// Event handlers call these instead of constructing data directly.
	// TypeScript checks parameters at the primitive level (string, boolean, ActiveTool[]).

	const emitToolExecute = (toolName: string, toolDetail: string | undefined, active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "tool.execute",
			tool_name: toolName,
			tool_detail: toolDetail,
			active_tools: active,
			is_busy: true,
		}, "tool.execute");

	const emitToolRunning = (toolName: string, toolDetail: string | undefined, active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "tool.running",
			event_emoji: "🔧",
			tool_name: toolName,
			tool_detail: toolDetail,
			active_tools: active,
			is_busy: true,
		}, "tool.running");

	const emitToolCompleted = (toolName: string, toolDetail: string | undefined, active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "tool.completed",
			tool_name: toolName,
			active_tools: active,
			is_busy: true,
		}, "tool.completed");

	const emitToolError = (toolName: string, toolDetail: string | undefined, active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "tool.error",
			tool_name: toolName,
			active_tools: active,
			is_busy: true,
		}, "tool.error");

	const emitSessionBusy = () =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "started",
			event_type: "session.busy",
			event_emoji: "🚀",
			active_tools: [],
			is_busy: true,
		}, "session.status");

	const emitSessionIdle = () =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "waiting",
			event_type: "session.idle",
			event_emoji: "💤",
			active_tools: [],
			is_busy: false,
		}, "session.status");

	const emitSessionCreated = (sessionId: string) =>
		sendStateDebounced({
			session_id: sessionId,
			status: "started",
			event_type: "session.created",
			event_emoji: "🚀",
			active_tools: [],
			is_busy: false,
		}, "session.created");

	const emitSessionDeleted = (sessionId: string) =>
		sendStateDebounced({
			session_id: sessionId,
			status: "ended",
			event_type: "session.deleted",
			event_emoji: "🏁",
			active_tools: [],
			is_busy: false,
		}, "session.deleted");

	const emitFileEdited = (filePath: string) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "file.edited",
			event_emoji: "📝",
			tool_name: "edit",
			tool_detail: filePath,
			active_tools: [{ key: { type: "opencode", name: "edit", detail: filePath }, name: "edit", detail: filePath }],
			is_busy: true,
		}, "file.edited");

	const emitPermissionAsked = (toolDetail: string | undefined, active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "permission",
			event_type: "permission.asked",
			event_emoji: "🔐",
			tool_detail: toolDetail,
			active_tools: active,
			is_busy: false,
		}, "permission.asked");

	const emitPermissionReplied = (active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "permission.replied",
			active_tools: active,
			is_busy: true,
		}, "permission.replied");

	const emitCommandExecuted = (commandDetail: string, active: ActiveTool[]) =>
		sendStateDebounced({
			session_id: currentSessionId || undefined,
			status: "running",
			event_type: "command.executed",
			tool_name: "bash",
			tool_detail: commandDetail,
			active_tools: active,
			is_busy: true,
		}, "command.executed");

	// ── Event handlers ───────────────────────────────────────────────

	return {
		"tool.execute.before": async (input: { tool: string; sessionID?: string }, output: { args: Record<string, unknown> }) => {
			const toolName = input.tool;
			const toolDetail = extractToolDetail(toolName, output.args);
			logRaw({ type: "tool.execute.before", tool: toolName, args: output.args, sessionID: input.sessionID });
			log("Tool executing", { tool: toolName, detail: toolDetail });

			if (input.sessionID) updateSession(input.sessionID);

			const tool: ActiveTool = {
				key: { type: "opencode", name: toolName, detail: toolDetail },
				name: toolName,
				detail: toolDetail,
			};
			if (!activeTools.some(t => t.key.name === toolName && t.key.detail === toolDetail)) {
				activeTools.push(tool);
			}

			emitToolExecute(toolName, toolDetail, [...activeTools]);
		},

		event: async ({ event }: { event: { type: string; properties?: any } }) => {
			const eventType = event.type;
			const props = event.properties || {};

			logRaw(event);
			log("Event received", { type: eventType });

			const sessionId = props.sessionID || props.session?.id || props.info?.id || props.info?.sessionID;
			if (sessionId) updateSession(sessionId);

			switch (eventType) {
				case "session.status": {
					const status = props.status;
					if (!status) return;

					log("Session status event", { statusType: status.type });

					if (status.type === "busy" && !isBusy) {
						isBusy = true;
						activeTools = [];
						emitSessionBusy();
					} else if (status.type === "idle" && isBusy) {
						isBusy = false;
						activeTools = [];
						emitSessionIdle();
					}
					break;
				}

				case "session.idle": {
					log("Session idle event");
					isBusy = false;
					activeTools = [];
					emitSessionIdle();
					break;
				}

				case "session.created": {
					const newSessionId = props.session?.id || props.id || sessionId;
					if (newSessionId) updateSession(newSessionId);
					emitSessionCreated(newSessionId || currentSessionId);
					break;
				}

				case "session.deleted": {
					if (props.sessionID === currentSessionId) {
						emitSessionDeleted(currentSessionId);
						currentSessionId = "";
						isBusy = false;
						activeTools = [];
					}
					break;
				}

				case "message.part.updated": {
					const part = props.part;
					if (!part || part.type !== "tool") return;

					const toolName = part.tool;
					const toolStatus = part.state?.status;
					const toolInput = part.state?.input;

					log("Tool part updated", { tool: toolName, status: toolStatus });

					if (toolStatus === "running") {
						const toolDetail = toolInput ? extractToolDetail(toolName, toolInput) : undefined;
						const tool: ActiveTool = {
							key: { type: "opencode", name: toolName, detail: toolDetail },
							name: toolName,
							detail: toolDetail,
						};
						if (!activeTools.some(t => t.key.name === toolName && t.key.detail === toolDetail)) {
							activeTools.push(tool);
						}
						emitToolRunning(toolName, toolDetail, [...activeTools]);
					} else if (toolStatus === "completed" || toolStatus === "error") {
						const toolDetail = toolInput ? extractToolDetail(toolName, toolInput) : undefined;
						activeTools = activeTools.filter(
							t => !(t.key.name === toolName && t.key.detail === toolDetail)
						);
						const removingTool: ActiveTool = {
							key: { type: "opencode", name: toolName, detail: toolDetail },
							name: toolName,
							detail: toolDetail,
						};
						const remaining = [removingTool, ...activeTools];
						if (toolStatus === "error") {
							emitToolError(toolName, toolDetail, remaining);
						} else {
							emitToolCompleted(toolName, toolDetail, remaining);
						}
					}
					break;
				}

				case "file.edited": {
					const filePath = props.path || props.filePath;
					if (!filePath) return;
					emitFileEdited(filePath);
					break;
				}

				case "permission.asked":
				case "permission.updated": {
					emitPermissionAsked(
						props.permission?.tool || props.permission?.type,
						[...activeTools],
					);
					break;
				}

				case "permission.replied": {
					if (isBusy) {
						emitPermissionReplied([...activeTools]);
					}
					break;
				}

				case "command.executed": {
					const command = props.command;
					if (Array.isArray(command) && command[0] === "bash") {
						emitCommandExecuted(command.slice(1).join(" "), [...activeTools]);
					}
					break;
				}
			}
		},
	};
};
