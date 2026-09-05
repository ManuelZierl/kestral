---
title: Tool servers
layout: default
parent: Using Kestral
nav_order: 3
---

# Connecting MCP tool servers
{: .no_toc }

1. TOC
{:toc}

Kestral can consume MCP servers over a local stdio process or MCP Streamable
HTTP. A saved server is normally only configuration: Kestral does not connect,
install, or grant it until you choose **Connect**.

A newly created profile includes one saved shortcut named **Kestral
documentation**, an unauthenticated GitMCP endpoint for the public Kestral
repository. It follows the same rule: startup makes no network request and does
not discover tools, install an app, or request a grant. Review the URL and choose
**Connect** only when you want to use it. The entry is an ordinary MCP server,
so you can edit or delete it without contacting the endpoint.

The current adapter requests MCP revision `2025-06-18` and also accepts a server
negotiating the earlier `2025-03-26` revision. Initialization must include server
identity, version, capabilities, and the tools capability. Every advertised tool
must provide an object-root `inputSchema`; an advertised `outputSchema`, available
from newer servers, must also have an object root. Malformed servers and revisions
outside this supported set fail the connection.

One HTTP, SSE, or stdio JSON-RPC message is limited to 8 MiB, and one complete
paginated tool discovery is limited to 16 MiB and 10,000 tools. Invalid cursors,
incomplete JSON-RPC responses, and results without a valid MCP content array fail
closed. Text-only results remain convenient JSON or text values; structured
content stays structured, and non-text content is retained as MCP content items
rather than reported as an empty success.

## Add and connect a server

1. Open **Settings → Tool servers**.
2. Choose **Add tool server**.
3. Enter a stable ID and display name.
4. Choose **Local command (stdio)** and enter the executable under **Command**
   plus one argument per line under **Arguments**, or choose **Remote endpoint
   (HTTP)** and enter its MCP URL. Spaces within one argument are preserved.
5. For an HTTP server, choose **None**, **Bearer token**, or **Custom secret
   header** under **HTTP authentication**. Save the server first, then enter the
   credential in the OS-backed field shown on its row.
6. Choose **Connect**.
7. Review the tools discovered during the MCP handshake and answer the
   trusted-chrome permission requests.

After connection, the server appears as an app. Its tools receive generic form
surfaces and result artifacts derived from their JSON Schemas. Simple scalar
properties receive individual controls. Arrays, nested objects, unions, and
other structured inputs use an explicitly labelled JSON-object editor with the
advertised schema available alongside it; Kestral never flattens those values
into misleading text fields. Calls require approval by default. Because bare
MCP metadata does not reliably describe side effects, generated capabilities
are `unspecified`; submitting their own generic forms therefore still opens
trusted chrome under that default policy. A packaged app can provide a custom
surface when the tool needs a guided end-user workflow.

Bearer authentication sends `Authorization: Bearer <credential>` on every MCP
HTTP request. Custom secret-header authentication accepts an HTTP header name
and an optional non-secret value prefix; the credential itself never enters
host config or frontend state. For example, X's app-only endpoint at
`https://api.x.com/mcp` uses the **Bearer token** option. Authentication failures
remain attached to that server row with **Retry connection**, so a final status
refresh does not erase the failure.

Static headers do not implement MCP OAuth discovery, browser authorization,
token refresh, mTLS, cookies, or request-signing schemes. Servers requiring
those flows need a compatible local bridge or future adapter support.

## Request access from Chat

Chat does not automatically receive every connected server tool. When a user
asks what MCP access is available, Chat can call the bundled **Permissions**
app's general read-only requestable-permission tool. It returns exact ungranted
capabilities from every installed provider, including currently connected MCP
providers, with bounded descriptions and effects. If no MCP entries appear,
connect a server first; an entirely empty result means no installed capability
is currently requestable.

After selecting an exact entry, Chat can create a proposal for that capability.
The proposal is a provenance-stamped artifact, not a grant, and changes no
authority by itself. The host constrains the proposal tool to the same bounded
general candidate set and does not supply it when the set is empty. Kestral does
not expose the MCP tools themselves as callable until their grants exist.

Review the proposal card in Chat and choose **Review and grant**. Kestral
revalidates the connected provider and capability, then shows the exact grant in
trusted chrome. Approval gives the requesting app access to that capability
with **Approval for delegated/high-impact use** as the default policy, so the
later tool call still requires its own decision.

The saved Kestral documentation shortcut follows this same flow. Connecting it
does not automatically give Chat access or open a second grant prompt. Request
only the exact documentation tools you want through the proposal and
trusted-chrome review above.

If another active permission already covers the capability with `notify` or
`silent`, the card reports that effective policy instead of claiming that every
call will ask. Use **Settings → Permissions** to change or revoke it.

This is the deliberately plain compatibility floor: a server author does not
need to know Kestral or write a manifest. A packaged app can progressively add
stable identity, custom surfaces, richer artifact types, agents, skills,
automations, configuration, and explicit integration grants without changing
the kernel model.

MCP tool results remain durable in **Artifacts**. Chat hides their inline cards
by default; enable **Settings → Chat → Conversation details → Show activity
details** to show compact result cards. **Open in Artifacts** opens, focuses, and
briefly highlights the complete result.

## Disconnect or change a server

Choose **Disconnect** on the generated app under **Apps**, or beside the server
under **Settings → Tool servers**, before editing or deleting configuration.
Disconnecting uninstalls the bridged app, revokes its grants, and shuts down the
transport.
The saved server can be connected again later, but Kestral does not reconnect
it on later startups. The seeded Kestral documentation shortcut follows the
same rule: every connection begins with an explicit **Connect** action.

Changing an authenticated server's endpoint or header configuration clears its
stored credential. Enter the credential again before reconnecting; Kestral does
not silently forward an existing secret to a changed destination.

## Security guidance

- A stdio command runs with your operating-system account and is not native-code
  sandboxed in the 0.1 series. Review the executable and arguments.
- Use HTTPS for remote servers outside the local machine.
- Kestral refuses authenticated plain HTTP except for loopback endpoints,
  rejects endpoint URLs containing user credentials, query parameters, or
  fragments, and does not follow HTTP redirects. Configure the final endpoint
  directly and keep credentials in the dedicated authentication setting.
- A remote endpoint is contacted from the host's network context and may
  intentionally be a loopback or private-network service. Connect only URLs
  you chose and trust; grants constrain tool calls, not where that server can
  respond from.
- Tool descriptions and read/write annotations are untrusted metadata. Grants
  and trusted-chrome decisions are the authority boundary.
- A permission proposal produced during an LLM or agent run is also untrusted
  until the host verifies its provenance and the user approves its exact grant
  through trusted chrome.
- A hung, crashing, malformed, or protocol-incompatible server fails its
  invocation or connection; it is not treated as successful.
- A transport credential may authorize more remote behavior than a Kestral
  grant exposes. Grants mediate calls through Kestral; they do not narrow the
  token's server-side scope or undo an external side effect.

Kestral supports MCP tools in the 0.1 series. MCP resources, prompts, and MCP Apps UI are
not imported.
