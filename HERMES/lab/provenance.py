"""Provenance-based trust boundary for LLM calls — SMCP run #4 baseline (v0).

The pattern-based sanitiser (Patches 1+7+8) catches ~82% of attacks but
9 attacks still slip through because they look benign at the lexical
level. The fundamental insight from indirect-prompt-injection research:
trust the SOURCE of content, not its surface text. If a piece of text
came from an MCP tool / file read / web fetch, ANY downstream LLM call
that is driven by it is intrinsically suspect.

This module provides a provenance-aware gate. The host LLM agent tags
every message in its context with a ProvenanceSource. When the agent
is about to issue a NEW LLM call (sampling/createMessage, recursive
chat), the gate inspects the trace and decides: BLOCK if any trigger
came from an untrusted source AND the user did not explicitly
authorise the tool path; ALLOW if the chain is grounded in user
intent.

This is run #4's baseline (v0). The Triad will iterate on edge cases
(propagation through tool chains, user authorisation semantics,
nested context, scope-limited authorisation, etc.) — see
tasks_run4.json.

The module is consumed by tools/mcp_tool.py (SamplingHandler) and by
agent/prompt_builder.py (when building messages from tool outputs).
For the lab harness it is also tested standalone via
run_provenance_lab.py against provenance_corpus.jsonl.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Iterable, Optional


class ProvenanceSource(str, Enum):
    """Where a message originated. The string values match log schemas."""
    USER = "user"            # direct user message (typed/dictated)
    SYSTEM = "system"        # built-in system prompt / config / SOUL.md
    ASSISTANT = "assistant"  # the host LLM's own prior output
    MCP_TOOL = "mcp_tool"    # output of an MCP server tool call
    FILE_READ = "file_read"  # content of a local file the agent read
    WEB_FETCH = "web_fetch"  # content fetched from a URL
    UNKNOWN = "unknown"      # source not annotated — treat as untrusted


# Trust partition. USER + SYSTEM are the only inherently-trusted
# sources. ASSISTANT is "intermediate trust": trusted as long as its
# parent context is trusted (no tool laundering).
_INHERENTLY_TRUSTED: frozenset[ProvenanceSource] = frozenset({
    ProvenanceSource.USER,
    ProvenanceSource.SYSTEM,
})

_INHERENTLY_UNTRUSTED: frozenset[ProvenanceSource] = frozenset({
    ProvenanceSource.MCP_TOOL,
    ProvenanceSource.FILE_READ,
    ProvenanceSource.WEB_FETCH,
    ProvenanceSource.UNKNOWN,
})

# Phrases that, in a USER message, are interpreted as authorising the
# next tool/LLM step. Conservative list — the user is expected to
# acknowledge each step explicitly.
_USER_AUTH_TOKENS: tuple[str, ...] = (
    "yes",
    "ok",
    "okay",
    "go ahead",
    "proceed",
    "allow",
    "do it",
    "approved",
    "authorise",
    "authorize",
    "confirmed",
    "confirm",
)


@dataclass(frozen=True)
class Tag:
    """Provenance tag attached to a single message."""
    source: ProvenanceSource
    server_name: Optional[str] = None    # e.g. "jina", "fs", "github"
    tool_name: Optional[str] = None
    parent: Optional["Tag"] = None       # propagation through chains

    def is_trusted(self) -> bool:
        """True iff this tag (and every ancestor) is inherently trusted."""
        if self.source in _INHERENTLY_UNTRUSTED:
            return False
        if self.parent is not None and not self.parent.is_trusted():
            return False
        return self.source in _INHERENTLY_TRUSTED or (
            self.source == ProvenanceSource.ASSISTANT
            and (self.parent is None or self.parent.is_trusted())
        )


@dataclass(frozen=True)
class Message:
    """A single message in a conversation trace."""
    role: str
    content: str
    tag: Tag


def _user_authorised(trace: Iterable[Message]) -> bool:
    """Return True if the LATEST user message contains an auth token.

    Conservative: only the most recent user message counts. A user must
    re-authorise each tool-derived chain.
    """
    user_msgs = [m for m in trace if m.tag.source == ProvenanceSource.USER]
    if not user_msgs:
        return False
    last = user_msgs[-1].content.lower()
    return any(token in last for token in _USER_AUTH_TOKENS)


def _untrusted_messages(trace: Iterable[Message]) -> list[Message]:
    return [m for m in trace if not m.tag.is_trusted()]


def should_block_llm_call(
    trace: list[Message],
    intent: str = "sampling",
) -> tuple[bool, str]:
    """Decide whether to BLOCK a fresh LLM call from the host agent.

    Args:
        trace: ordered list of messages forming the prompt context.
        intent: "sampling" (MCP-initiated), "tool" (chained tool call),
                "user_chat" (direct continuation of the user prompt),
                or other custom intent strings.

    Returns:
        (block, reason). block=True means the gate STOPS the call.
    """
    if not trace:
        # Empty context — nothing to inspect; default to ALLOW (the
        # caller will provide its own validation).
        return False, ""

    untrusted = _untrusted_messages(trace)
    if not untrusted:
        return False, ""  # all-trusted trace, allow

    # MCP-initiated sampling is the worst case: the request is *driven*
    # by an MCP server. ALWAYS block unless the user explicitly
    # authorised it in this turn.
    if intent == "sampling":
        if _user_authorised(trace):
            return False, ""
        sources = sorted({m.tag.source.value for m in untrusted})
        return True, (
            f"sampling/createMessage blocked: trace contains untrusted "
            f"sources {sources} and no user authorisation in latest turn"
        )

    # Tool-chain calls: allow only if the user's latest message
    # explicitly authorised the chain.
    if intent == "tool":
        if _user_authorised(trace):
            return False, ""
        sources = sorted({m.tag.source.value for m in untrusted})
        return True, (
            f"tool-chain blocked: untrusted sources {sources} without "
            f"user authorisation"
        )

    # Direct user-driven continuation: allow as long as the user
    # message itself is the most recent in the trace.
    if intent == "user_chat":
        last_msg = trace[-1]
        if last_msg.tag.source == ProvenanceSource.USER:
            return False, ""
        return True, (
            f"user_chat blocked: last message is from "
            f"{last_msg.tag.source.value}, not user"
        )

    # Unknown intent — fail closed.
    return True, f"unknown intent {intent!r} — failing closed"


def tag_user(content: str) -> Tag:
    return Tag(source=ProvenanceSource.USER)


def tag_system(content: str) -> Tag:
    return Tag(source=ProvenanceSource.SYSTEM)


def tag_assistant(content: str, parent: Optional[Tag] = None) -> Tag:
    return Tag(source=ProvenanceSource.ASSISTANT, parent=parent)


def tag_mcp_tool(content: str, server_name: str,
                 tool_name: Optional[str] = None) -> Tag:
    return Tag(source=ProvenanceSource.MCP_TOOL,
               server_name=server_name, tool_name=tool_name)


def tag_file_read(content: str, path: str) -> Tag:
    return Tag(source=ProvenanceSource.FILE_READ, server_name=path)


def tag_web_fetch(content: str, url: str) -> Tag:
    return Tag(source=ProvenanceSource.WEB_FETCH, server_name=url)
