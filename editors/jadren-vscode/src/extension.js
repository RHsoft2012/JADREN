const vscode = require('vscode');
const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');
const https = require('https');
const { spawn } = require('child_process');
const {
  LanguageClient,
  TransportKind,
  CloseAction,
  ErrorAction,
} = require('vscode-languageclient/node');
const {
  activeCall: localActiveCall,
  buildLocalModel,
  dotContext,
  membersForDot,
} = require('./offlineTools');

let client;
let debugOutput;
let buildOutput;
let lspOutput;
let updateTimer;
let updateInterval;
let updateInFlight = false;

const OFFLINE_COMPLETIONS = [
  ['Bool', vscode.CompletionItemKind.Type, 'Boolean type'],
  ['Int32', vscode.CompletionItemKind.Type, '32-bit signed integer type'],
  ['Int64', vscode.CompletionItemKind.Type, '64-bit signed integer type'],
  ['UInt32', vscode.CompletionItemKind.Type, '32-bit unsigned integer type'],
  ['Float32', vscode.CompletionItemKind.Type, '32-bit floating-point type'],
  ['Float64', vscode.CompletionItemKind.Type, '64-bit floating-point type'],
  ['String', vscode.CompletionItemKind.Type, 'UTF-8 string type'],
  ['OwnedString', vscode.CompletionItemKind.Type, 'Move-only owning UTF-8 string type'],
  ['Vec2', vscode.CompletionItemKind.Type, '2D vector type'],
  ['Vec3', vscode.CompletionItemKind.Type, '3D vector type'],
  ['Vec4', vscode.CompletionItemKind.Type, '4D vector type'],
  ['print', vscode.CompletionItemKind.Function, 'Print a value to stdout'],
  ['stdin_read', vscode.CompletionItemKind.Function, 'Read bounded bytes from native standard input'],
  ['stdout_write', vscode.CompletionItemKind.Function, 'Write caller-owned bytes to native standard output'],
  ['stderr_write', vscode.CompletionItemKind.Function, 'Write caller-owned bytes to native standard error'],
  ['time_now_unix_seconds', vscode.CompletionItemKind.Function, 'Return current UTC Unix time in seconds'],
  ['time_now_monotonic_ms', vscode.CompletionItemKind.Function, 'Return monotonic elapsed time in milliseconds'],
  ['process_arg_count', vscode.CompletionItemKind.Function, 'Return the number of native process arguments'],
  ['process_arg_read', vscode.CompletionItemKind.Function, 'Copy one UTF-8 process argument into a caller-owned buffer'],
  ['time_utc_parts', vscode.CompletionItemKind.Function, 'Decompose Unix seconds into UTC calendar fields'],
  ['time_utc_offset_parts', vscode.CompletionItemKind.Function, 'Apply an explicit fixed offset and decompose UTC calendar fields'],
  ['app_scheduler_clear', vscode.CompletionItemKind.Function, 'Clear the bounded application timer queue'],
  ['app_scheduler_set', vscode.CompletionItemKind.Function, 'Insert or replace a caller-driven application timer'],
  ['app_scheduler_cancel', vscode.CompletionItemKind.Function, 'Cancel an application timer by task id'],
  ['app_scheduler_poll', vscode.CompletionItemKind.Function, 'Poll due application timer ids into a bounded slice'],
  ['app_scheduler_count', vscode.CompletionItemKind.Function, 'Return the number of active application timers'],
  ['buffer_create', vscode.CompletionItemKind.Function, 'Create a generic owning Buffer<T> from the expected result type'],
  ['buffer_clear', vscode.CompletionItemKind.Function, 'Clear a copy-safe Buffer<T> without releasing its allocation'],
  ['buffer_clear_status', vscode.CompletionItemKind.Function, 'Return a bounded status code while clearing a copy-safe Buffer<T>'],
  ['buffer_clear_move', vscode.CompletionItemKind.Function, 'Clear nested buffers, owning records, or Buffer<OwnedString> with destructor-aware cleanup'],
  ['buffer_clear_move_status', vscode.CompletionItemKind.Function, 'Return a bounded status code for nested, record, or OwnedString-buffer clear'],
  ['buffer_reserve', vscode.CompletionItemKind.Function, 'Grow a generic Buffer<T>; move-only record fields use compiler-generated drop glue'],
  ['buffer_append', vscode.CompletionItemKind.Function, 'Append a scalar, copy-safe nominal, or owning @repr(C) record value'],
  ['buffer_append_status', vscode.CompletionItemKind.Function, 'Return a bounded status code for generic Buffer<T> append'],
  ['buffer_resize_move', vscode.CompletionItemKind.Function, 'Resize an owning nested buffer, record Buffer, or Buffer<OwnedString> with cleanup'],
  ['buffer_resize_move_status', vscode.CompletionItemKind.Function, 'Return a status code for nested, record, or OwnedString-buffer resize'],
  ['buffer_insert_move', vscode.CompletionItemKind.Function, 'Move an owning nested Buffer<U> into a bounded Buffer<Buffer<U>>'],
  ['buffer_insert_move_status', vscode.CompletionItemKind.Function, 'Return a status code for a move-only nested buffer insert'],
  ['buffer_insert_move_from', vscode.CompletionItemKind.Function, 'Insert any move-safe owning T from a caller-owned source slot'],
  ['buffer_insert_move_from_status', vscode.CompletionItemKind.Function, 'Return a status code for a caller-owned generic move insert'],
  ['buffer_append_move', vscode.CompletionItemKind.Function, 'Move any move-safe owning T to the end of a Buffer<T> and zero the source'],
  ['buffer_append_move_status', vscode.CompletionItemKind.Function, 'Return a status code for a caller-owned generic move append'],
  ['buffer_pop', vscode.CompletionItemKind.Function, 'Move the last element out of Buffer<Buffer<U>> or Buffer<OwnedString>'],
  ['buffer_pop_move_into', vscode.CompletionItemKind.Function, 'Move the last move-safe owning Buffer<T> element into a caller-owned output slot'],
  ['buffer_pop_move_into_status', vscode.CompletionItemKind.Function, 'Return a status code for a last-element move into a caller-owned slot'],
  ['buffer_remove_move', vscode.CompletionItemKind.Function, 'Move one element out of Buffer<Buffer<U>> or Buffer<OwnedString>'],
  ['buffer_remove_move_into', vscode.CompletionItemKind.Function, 'Move any move-safe owning Buffer<T> element into a caller-owned output slot'],
  ['buffer_remove_move_into_status', vscode.CompletionItemKind.Function, 'Return a status code for a move-safe owning Buffer<T> remove'],
  ['buffer_remove_drop', vscode.CompletionItemKind.Function, 'Destroy one copy-safe or OwnedString element and compact the buffer'],
  ['buffer_remove_drop_status', vscode.CompletionItemKind.Function, 'Return a status code for copy-safe or OwnedString drop-remove'],
  ['http_response_status', vscode.CompletionItemKind.Function, 'Read the HTTP response status code'],
  ['http_response_status_prefix', vscode.CompletionItemKind.Function, 'Read the status code from an explicit response prefix'],
  ['http_response_header', vscode.CompletionItemKind.Function, 'Copy one HTTP response header into a caller-owned buffer'],
  ['http_response_header_prefix', vscode.CompletionItemKind.Function, 'Copy one header from an explicit response prefix'],
  ['http_response_header_exact', vscode.CompletionItemKind.Function, 'Read one response header with an explicit output length'],
  ['http_response_body', vscode.CompletionItemKind.Function, 'Copy a bounded HTTP response body'],
  ['http_response_body_prefix', vscode.CompletionItemKind.Function, 'Copy a response body from an explicit response prefix'],
  ['http_response_body_exact', vscode.CompletionItemKind.Function, 'Read a response body with an explicit output length'],
  ['http_response_body_chunked_exact', vscode.CompletionItemKind.Function, 'Decode a complete Transfer-Encoding: chunked response with an explicit output length'],
  ['http_request_body_chunked_exact', vscode.CompletionItemKind.Function, 'Decode a complete Transfer-Encoding: chunked request with an explicit output length'],
  ['http_response_write_ex', vscode.CompletionItemKind.Function, 'Serialize an HTTP response with explicit keep-alive mode'],
  ['http_response_write_header', vscode.CompletionItemKind.Function, 'Serialize an HTTP response with one custom header'],
  ['http_response_write_header_ex', vscode.CompletionItemKind.Function, 'Serialize an HTTP response with one custom header and keep-alive mode'],
  ['http_response_write_header_block', vscode.CompletionItemKind.Function, 'Serialize an HTTP response with a validated custom-header block'],
  ['http_response_write_header_block_ex', vscode.CompletionItemKind.Function, 'Serialize an HTTP response with a custom-header block and keep-alive mode'],
  ['http_response_write_cookie', vscode.CompletionItemKind.Function, 'Serialize an HTTP response with a validated dynamic Set-Cookie header'],
  ['http_response_write_cookie_ex', vscode.CompletionItemKind.Function, 'Serialize a dynamic Set-Cookie response with keep-alive mode'],
  ['http_request_write_header_block', vscode.CompletionItemKind.Function, 'Serialize an HTTP request with a validated custom-header block'],
  ['bearer_token_matches', vscode.CompletionItemKind.Function, 'Match a caller-owned bearer token without storing auth state'],
  ['cookie_value_matches', vscode.CompletionItemKind.Function, 'Match one exact name=value pair in a Cookie header'],
  ['http_request_append', vscode.CompletionItemKind.Function, 'Append a bounded received fragment into an HTTP request buffer'],
  ['http_request_is_complete_prefix', vscode.CompletionItemKind.Function, 'Validate an HTTP request prefix inside a larger buffer'],
  ['http_request_frame_length_prefix', vscode.CompletionItemKind.Function, 'Return the exact first HTTP request frame length'],
  ['http_request_chunked_frame_length_prefix', vscode.CompletionItemKind.Function, 'Return the exact first chunked HTTP request frame length'],
  ['http_request_consume_prefix', vscode.CompletionItemKind.Function, 'Remove one parsed HTTP frame and retain the remaining prefix'],
  ['http_route_match_prefix', vscode.CompletionItemKind.Function, 'Match an HTTP route using an explicit request length'],
  ['http_request_keep_alive', vscode.CompletionItemKind.Function, 'Read the HTTP/1.1 Connection keep-alive policy'],
  ['rate_limit_allow', vscode.CompletionItemKind.Function, 'Allow a request using caller-owned fixed-window state'],
  ['rate_limit_retry_after_ms', vscode.CompletionItemKind.Function, 'Read the remaining caller-owned rate-limit window'],
  ['http_router_clear', vscode.CompletionItemKind.Function, 'Clear the bounded HTTP route table'],
  ['http_router_add', vscode.CompletionItemKind.Function, 'Insert or replace an exact HTTP route'],
  ['http_router_add_prefix', vscode.CompletionItemKind.Function, 'Insert or replace a bounded prefix HTTP route'],
  ['http_router_remove', vscode.CompletionItemKind.Function, 'Remove one exact HTTP route if it exists'],
  ['http_router_remove_prefix', vscode.CompletionItemKind.Function, 'Remove one bounded prefix HTTP route if it exists'],
  ['http_router_respond', vscode.CompletionItemKind.Function, 'Serialize a bounded response for an HTTP request'],
  ['http_router_respond_prefix', vscode.CompletionItemKind.Function, 'Serialize a bounded response using an explicit request length'],
  ['http_router_count', vscode.CompletionItemKind.Function, 'Return the number of active HTTP routes'],
  ['http_session_open', vscode.CompletionItemKind.Function, 'Open a bounded HTTP session over a TCP listener'],
  ['http_session_open_tls', vscode.CompletionItemKind.Function, 'Open a bounded HTTP session with native TLS credentials'],
  ['http_session_step', vscode.CompletionItemKind.Function, 'Run one bounded HTTP session step'],
  ['http_session_close', vscode.CompletionItemKind.Function, 'Close an HTTP session and all owned connections'],
  ['net_tls_open_client', vscode.CompletionItemKind.Function, 'Open a native TLS client over an existing TCP socket'],
  ['net_tls_open_server', vscode.CompletionItemKind.Function, 'Open a native TLS server over an accepted TCP socket'],
  ['net_tls_step', vscode.CompletionItemKind.Function, 'Advance the bounded TLS handshake'],
  ['net_tls_state', vscode.CompletionItemKind.Function, 'Read the TLS connection state'],
  ['net_tls_error', vscode.CompletionItemKind.Function, 'Read the last native TLS error code'],
  ['net_tls_send', vscode.CompletionItemKind.Function, 'Encrypt and send one bounded TLS payload'],
  ['net_tls_receive', vscode.CompletionItemKind.Function, 'Receive and decrypt one bounded TLS payload'],
  ['net_tls_close', vscode.CompletionItemKind.Function, 'Close the TLS context and owned TCP socket'],
  ['net_reactor_open', vscode.CompletionItemKind.Function, 'Open a bounded native readiness reactor'],
  ['net_reactor_watch', vscode.CompletionItemKind.Function, 'Watch a socket for readable or writable events'],
  ['net_reactor_unwatch', vscode.CompletionItemKind.Function, 'Cancel one reactor socket watch'],
  ['net_reactor_poll', vscode.CompletionItemKind.Function, 'Wait for bounded socket readiness events'],
  ['net_reactor_event_socket', vscode.CompletionItemKind.Function, 'Read the socket token from a reactor event'],
  ['net_reactor_event_flags', vscode.CompletionItemKind.Function, 'Read readiness flags from a reactor event'],
  ['net_reactor_event_bytes', vscode.CompletionItemKind.Function, 'Read bytes transferred by a buffered reactor operation'],
  ['net_reactor_event_user', vscode.CompletionItemKind.Function, 'Read caller user data from a reactor event'],
  ['net_reactor_error', vscode.CompletionItemKind.Function, 'Read the last native reactor error'],
  ['net_reactor_close', vscode.CompletionItemKind.Function, 'Close a reactor without closing watched sockets'],
  ['net_reactor_submit_accept', vscode.CompletionItemKind.Function, 'Submit a bounded asynchronous accept operation'],
  ['net_reactor_submit_connect', vscode.CompletionItemKind.Function, 'Submit a bounded asynchronous IPv4 connect operation'],
  ['net_reactor_submit_receive', vscode.CompletionItemKind.Function, 'Submit a zero-copy receive readiness operation'],
  ['net_reactor_submit_send', vscode.CompletionItemKind.Function, 'Submit a zero-copy send readiness operation'],
  ['net_reactor_submit_receive_buffer', vscode.CompletionItemKind.Function, 'Submit a caller-buffered asynchronous receive operation'],
  ['net_reactor_submit_send_buffer', vscode.CompletionItemKind.Function, 'Submit a caller-buffered asynchronous send operation'],
  ['net_reactor_submit_send_buffer_prefix', vscode.CompletionItemKind.Function, 'Submit an exact-prefix caller-buffered asynchronous send operation'],
  ['net_reactor_cancel', vscode.CompletionItemKind.Function, 'Cancel a pending reactor operation'],
  ['net_reactor_event_operation', vscode.CompletionItemKind.Function, 'Read the operation token from a reactor event'],
  ['string_length', vscode.CompletionItemKind.Function, 'Return UTF-8 string length in bytes'],
  ['string_equals', vscode.CompletionItemKind.Function, 'Compare two UTF-8 strings without allocation'],
  ['string_builder_append', vscode.CompletionItemKind.Function, 'Append a String into a bounded byte buffer'],
  ['string_builder_append_bytes', vscode.CompletionItemKind.Function, 'Append bytes into a bounded byte buffer'],
  ['string_owned_create', vscode.CompletionItemKind.Function, 'Create an owning UTF-8 string with a capacity'],
  ['string_owned_from', vscode.CompletionItemKind.Function, 'Copy a borrowed String into an owning UTF-8 string'],
  ['string_owned_append', vscode.CompletionItemKind.Function, 'Append UTF-8 text to an owning string'],
  ['string_owned_length', vscode.CompletionItemKind.Function, 'Return the byte length of an owning string'],
  ['string_owned_copy', vscode.CompletionItemKind.Function, 'Copy an owning string into a bounded byte buffer'],
  ['string_owned_clear', vscode.CompletionItemKind.Function, 'Clear an owning string while retaining capacity'],
  ['fn', vscode.CompletionItemKind.Keyword, 'Declare a function'],
  ['let', vscode.CompletionItemKind.Keyword, 'Declare a local binding'],
  ['if', vscode.CompletionItemKind.Keyword, 'Conditional block'],
  ['else', vscode.CompletionItemKind.Keyword, 'Alternative conditional block'],
  ['for', vscode.CompletionItemKind.Keyword, 'Iteration block'],
  ['while', vscode.CompletionItemKind.Keyword, 'While loop'],
  ['return', vscode.CompletionItemKind.Keyword, 'Return from a function'],
  ['module', vscode.CompletionItemKind.Keyword, 'Declare a module'],
  ['import', vscode.CompletionItemKind.Keyword, 'Import a module'],
  ['struct', vscode.CompletionItemKind.Keyword, 'Declare a struct'],
  ['enum', vscode.CompletionItemKind.Keyword, 'Declare an enum'],
  ['match', vscode.CompletionItemKind.Keyword, 'Pattern matching expression'],
];

const OFFLINE_HOVER_DOCS = new Map([
  ['fn', '**fn** declares a Jadren function. Example: `fn add(a: Int32, b: Int32) -> Int32 { ... }`'],
  ['let', '**let** declares a local binding with an explicit or inferred type.'],
  ['if', '**if** evaluates a condition and executes its block when the condition is true.'],
  ['else', '**else** provides the alternative branch of an `if` expression.'],
  ['for', '**for** iterates over a range or collection. Example: `for item in items { ... }`'],
  ['while', '**while** repeats a block while its condition remains true.'],
  ['return', '**return** finishes the current function and returns a value when required.'],
  ['module', '**module** declares the module name at the top of a Jadren source file.'],
  ['import', '**import** makes public declarations from another module available.'],
  ['struct', '**struct** declares a product type with named fields.'],
  ['enum', '**enum** declares a type with a fixed set of variants.'],
  ['match', '**match** selects a branch using pattern matching.'],
  ['print', '**print** writes a value to standard output.'],
  ['stdin_read', '**stdin_read** reads at most the capacity of a caller-owned `write Slice<UInt8>` from native standard input and returns the byte count.'],
  ['stdout_write', '**stdout_write** writes the complete caller-owned `read Slice<UInt8>` to native standard output and returns the byte count.'],
  ['stderr_write', '**stderr_write** writes the complete caller-owned `read Slice<UInt8>` to native standard error and returns the byte count.'],
  ['time_now_unix_seconds', '**time_now_unix_seconds** returns the current UTC Unix timestamp in seconds as `Int64`.'],
  ['time_now_monotonic_ms', '**time_now_monotonic_ms** returns a monotonic elapsed-time counter in milliseconds as `UInt64`.'],
  ['process_arg_count', '**process_arg_count** returns the argument count, including executable path at index `0`, as `UIntSize`.'],
  ['process_arg_read', '**process_arg_read** copies one native process argument as UTF-8 into a caller-owned `write Slice<UInt8>`; invalid indices or insufficient capacity return `0` without partial output.'],
  ['time_utc_parts', '**time_utc_parts** writes `[year, month, day, hour, minute, second]` into a bounded `write Slice<Int32>` using UTC only.'],
  ['time_utc_offset_parts', '**time_utc_offset_parts** applies an explicit fixed offset in minutes before writing UTC calendar fields; it never reads the host timezone or DST policy.'],
  ['app_scheduler_clear', '**app_scheduler_clear** removes all entries from the fixed-size, caller-driven application timer queue.'],
  ['app_scheduler_set', '**app_scheduler_set** inserts or replaces a timer; `repeat_seconds == 0` creates a one-shot timer.'],
  ['app_scheduler_cancel', '**app_scheduler_cancel** removes one timer by its `task_id` without creating a callback or worker.'],
  ['app_scheduler_poll', '**app_scheduler_poll** writes due task IDs in stable `(due, task_id)` order; a small output slice leaves the queue unchanged.'],
  ['http_request_append', '**http_request_append** appends a bounded received fragment into a caller-owned request buffer and returns the new length; invalid capacity or prefix lengths leave the buffer unchanged.'],
  ['http_request_is_complete_prefix', '**http_request_is_complete_prefix** validates only the first input_length bytes of a larger caller-owned buffer.'],
  ['http_request_frame_length_prefix', '**http_request_frame_length_prefix** returns the exact first HTTP/1.1 request boundary, including Content-Length bytes; `0` means invalid or incomplete. Transfer-Encoding is rejected.'],
  ['http_request_chunked_frame_length_prefix', '**http_request_chunked_frame_length_prefix** returns the exact first complete `Transfer-Encoding: chunked` request boundary from a received prefix; `0` means invalid or incomplete, and any pipelined suffix remains untouched.'],
  ['http_request_consume_prefix', '**http_request_consume_prefix** shifts the unconsumed bytes to the beginning of a caller-owned buffer and returns the remaining length.'],
  ['http_route_match_prefix', '**http_route_match_prefix** matches method and target against only the first input_length bytes of a larger caller-owned request buffer.'],
  ['app_scheduler_count', '**app_scheduler_count** returns the number of active entries in the bounded timer queue.'],
  ['http_response_write_ex', '**http_response_write_ex** serializes a response with an explicit `keep_alive` boolean and never performs socket I/O.'],
  ['http_response_write_header', '**http_response_write_header** serializes a bounded response with one validated custom header, such as `Retry-After`, and never performs socket I/O.'],
  ['http_response_write_header_ex', '**http_response_write_header_ex** adds one validated custom header and an explicit `keep_alive` boolean to the bounded response writer.'],
  ['http_response_write_header_block', '**http_response_write_header_block** serializes a bounded response with a CRLF-separated block of validated custom headers; framing headers and malformed lines are rejected before writing.'],
  ['http_response_write_header_block_ex', '**http_response_write_header_block_ex** adds a validated custom-header block and an explicit `keep_alive` boolean to the bounded response writer.'],
  ['http_response_write_cookie', '**http_response_write_cookie** validates a dynamic cookie name, value and attribute suffix, then emits one `Set-Cookie` response header without partial writes.'],
  ['http_response_write_cookie_ex', '**http_response_write_cookie_ex** emits the same validated dynamic `Set-Cookie` header with an explicit `keep_alive` boolean.'],
  ['http_response_status', '**http_response_status** validates an HTTP/1.1 status line and returns its three-digit status code, or `0` for invalid input.'],
  ['http_response_status_prefix', '**http_response_status_prefix** reads only the explicit valid prefix of a larger caller-owned receive buffer.'],
  ['http_response_header', '**http_response_header** copies one case-insensitive response header into a caller-owned output slice.'],
  ['http_response_header_prefix', '**http_response_header_prefix** copies one response header while separating receive capacity from valid input length.'],
  ['http_response_header_exact', '**http_response_header_exact** returns `Bool`, writes the decoded header length separately, and preserves a valid empty value.'],
  ['http_response_body', '**http_response_body** validates `Content-Length` and copies the complete response body without chunked transfer.'],
  ['http_response_body_prefix', '**http_response_body_prefix** reads a response body only after validating the explicit input prefix and output capacity.'],
  ['http_response_body_exact', '**http_response_body_exact** returns `Bool`, writes `Content-Length` separately, and distinguishes an empty body from invalid framing.'],
  ['http_response_body_chunked_exact', '**http_response_body_chunked_exact** decodes a complete `Transfer-Encoding: chunked` response, writes the decoded length separately, and never partially writes on invalid framing or insufficient capacity.'],
  ['http_request_body_chunked_exact', '**http_request_body_chunked_exact** decodes a complete `Transfer-Encoding: chunked` request, writes the decoded length separately, and never partially writes on invalid framing or insufficient capacity.'],
  ['http_request_write_header_block', '**http_request_write_header_block** serializes a request with multiple validated CRLF-separated custom headers; framing names and malformed lines are rejected before writing.'],
  ['bearer_token_matches', '**bearer_token_matches** compares an exact `Bearer ` token in caller-owned scratch memory without storing authentication state.'],
  ['cookie_value_matches', '**cookie_value_matches** finds one exact `name=value` pair in a `Cookie` header and performs no allocation or session-state mutation.'],
  ['http_request_keep_alive', '**http_request_keep_alive** returns `true` for a valid HTTP/1.1 request without `Connection: close`, and `false` for close or unsupported modes.'],
  ['http_router_clear', '**http_router_clear** removes all entries from the fixed-size HTTP route table.'],
  ['http_router_add', '**http_router_add** inserts or replaces an exact `(method, target)` route in the 16-slot table.'],
  ['http_router_add_prefix', '**http_router_add_prefix** inserts or replaces a prefix route with exact-over-prefix dispatch priority.'],
  ['http_router_remove', '**http_router_remove** removes one exact `(method, target)` route and returns `false` when it is absent.'],
  ['http_router_remove_prefix', '**http_router_remove_prefix** removes one exact `(method, target_prefix)` prefix route.'],
  ['http_router_respond', '**http_router_respond** parses the request line and writes the configured response, or a bounded 404.'],
  ['http_router_respond_prefix', '**http_router_respond_prefix** parses only the explicit valid request prefix before writing the configured response or bounded 404.'],
  ['http_router_count', '**http_router_count** returns the number of active bounded HTTP routes.'],
  ['rate_limit_allow', '**rate_limit_allow** accepts a request in a caller-owned fixed window; the two-element state is never shared or allocated by the policy.'],
  ['rate_limit_retry_after_ms', '**rate_limit_retry_after_ms** returns the remaining milliseconds in a caller-owned rate window without mutating it.'],
  ['http_session_open', '**http_session_open** creates a fixed-slot HTTP session over a listener; header/body limits are validated before any connection is accepted.'],
  ['http_session_open_tls', '**http_session_open_tls** creates the same bounded HTTP session with a native TLS server credential pair; Windows expects PFX plus password and Linux expects PEM certificate plus private key.'],
  ['http_session_step', '**http_session_step** performs one bounded accept/read/route/write step. It returns `1` for a keep-alive response, `2` for a terminal connection event, and `0` for idle/incomplete work.'],
  ['http_session_close', '**http_session_close** closes all session-owned connections and the listener with cancel-safe cleanup.'],
  ['net_tls_open_client', '**net_tls_open_client** creates a native TLS client over an existing TCP socket. The explicit `verify_peer` flag must be `true` for production validation; `false` is intended only for local self-signed fixtures.'],
  ['net_tls_open_server', '**net_tls_open_server** creates a bounded TLS server over an accepted TCP socket. Windows expects a PFX certificate path plus password; Linux expects PEM certificate and private-key paths.'],
  ['net_tls_step', '**net_tls_step** advances the platform handshake: `1` means more I/O is needed, `2` established, `3` peer closed, and `4` error.'],
  ['net_tls_state', '**net_tls_state** returns `0` closed, `1` handshaking, `2` open, `3` error, or `4` peer closed.'],
  ['net_tls_error', '**net_tls_error** returns the last native TLS status code for diagnostics.'],
  ['net_tls_send', '**net_tls_send** encrypts and sends one bounded payload after the handshake is open.'],
  ['net_tls_receive', '**net_tls_receive** decrypts one bounded application payload into a caller-owned output slice.'],
  ['net_tls_close', '**net_tls_close** releases the native TLS context and closes its owned TCP socket.'],
  ['net_reactor_open', '**net_reactor_open** creates a bounded native readiness reactor. `max_watches` and `max_events` are capped at 64 and the token owns only reactor state, not sockets.'],
  ['net_reactor_watch', '**net_reactor_watch** registers or updates a socket with interest bits `1=readable`, `2=writable`, `4=error`, `8=peer closed` and caller `user_data`.'],
  ['net_reactor_unwatch', '**net_reactor_unwatch** removes one socket watch; it is the cancel-safe cleanup operation and does not close the socket.'],
  ['net_reactor_poll', '**net_reactor_poll** waits up to `timeout_ms` and returns the number of bounded events. `0` means timeout or no event.'],
  ['net_reactor_event_socket', '**net_reactor_event_socket** returns the socket token for an event index from the most recent poll.'],
  ['net_reactor_event_flags', '**net_reactor_event_flags** returns readiness bits for an event index from the most recent poll.'],
  ['net_reactor_event_bytes', '**net_reactor_event_bytes** returns the number of bytes transferred by a buffered operation; readiness watch events return `0`.'],
  ['net_reactor_event_user', '**net_reactor_event_user** returns the caller `user_data` associated with an event.'],
  ['net_reactor_error', '**net_reactor_error** returns the last native reactor error code, or `-1` for an invalid reactor.'],
  ['net_reactor_close', '**net_reactor_close** releases reactor state while leaving watched sockets owned by the caller.'],
  ['net_reactor_submit_accept', '**net_reactor_submit_accept** submits a bounded asynchronous accept. The event returns the newly accepted socket token.'],
  ['net_reactor_submit_connect', '**net_reactor_submit_connect** submits an asynchronous IPv4 connect without retaining caller memory. The event returns the connected socket token.'],
  ['net_reactor_submit_receive', '**net_reactor_submit_receive** arms one receive-readiness operation; call `net_tcp_receive` after the event with your caller-owned buffer.'],
  ['net_reactor_submit_send', '**net_reactor_submit_send** arms one send-readiness operation; call `net_tcp_send` after the event with your caller-owned buffer.'],
  ['net_reactor_submit_receive_buffer', '**net_reactor_submit_receive_buffer** submits a native buffered receive into a caller-owned `write Slice<UInt8>`; read `net_reactor_event_bytes` after completion.'],
  ['net_reactor_submit_send_buffer', '**net_reactor_submit_send_buffer** submits a native buffered send from a caller-owned `read Slice<UInt8>`; read `net_reactor_event_bytes` after completion.'],
  ['net_reactor_submit_send_buffer_prefix', '**net_reactor_submit_send_buffer_prefix** submits exactly `length` bytes from a caller-owned `read Slice<UInt8>`; the length must not exceed the slice and completed bytes are available through `net_reactor_event_bytes`.'],
  ['net_reactor_cancel', '**net_reactor_cancel** requests cancellation of a pending operation. Completion is still reported as an error event when the platform delivers it.'],
  ['net_reactor_event_operation', '**net_reactor_event_operation** returns the one-shot operation token for a completion event, or `0` for a readiness watch event.'],
  ['string_length', '**string_length** returns the UTF-8 byte length of a `String` as `UIntSize`.'],
  ['string_equals', '**string_equals** compares two `String` values without allocating.'],
  ['string_builder_append', '**string_builder_append** copies a `String` at an offset in a caller-owned byte buffer and returns the new offset.'],
  ['string_builder_append_bytes', '**string_builder_append_bytes** copies a byte slice at an offset in a caller-owned buffer and returns the new offset.'],
  ['string_owned_create', '**string_owned_create** returns `Result<OwnedString, Int32>` with the requested capacity.'],
  ['string_owned_from', '**string_owned_from** validates and copies a `String` into `Result<OwnedString, Int32>`.'],
  ['string_owned_append', '**string_owned_append** appends UTF-8 text to a move-only `OwnedString` and returns a status code.'],
  ['string_owned_length', '**string_owned_length** returns the UTF-8 byte length of an `OwnedString` as `UIntSize`.'],
  ['string_owned_copy', '**string_owned_copy** copies an `OwnedString` into a caller-owned bounded byte slice and returns bytes written.'],
  ['string_owned_clear', '**string_owned_clear** clears an `OwnedString` without releasing its reserved capacity.'],
  ['buffer_insert_move', '**buffer_insert_move** moves an owning `Buffer<U>` or `OwnedString` into a descriptor-backed buffer at the requested index. A successful move clears the source descriptor; an invalid index leaves it unchanged.'],
  ['buffer_insert_move_status', '**buffer_insert_move_status** is the status-code form of `buffer_insert_move`; `0` is success and `-20` means an invalid index.'],
  ['buffer_insert_move_from', '**buffer_insert_move_from** inserts any already-supported move-safe owning `T` (nested Buffer, carrier, owning `@repr(C)` enum/record, or fixed owning array) from a caller-owned source slot. On success the source bytes are zeroed; validation or allocation failure leaves source and destination unchanged.'],
  ['buffer_insert_move_from_status', '**buffer_insert_move_from_status** is the status-code form of `buffer_insert_move_from`; `0` is success and the source remains unchanged on failure.'],
  ['buffer_append_move', '**buffer_append_move** appends any already-supported move-safe owning `T` from a caller-owned source slot. On success the source bytes are zeroed; allocation or validation failure leaves source and destination unchanged.'],
  ['buffer_append_move_status', '**buffer_append_move_status** is the status-code form of `buffer_append_move`; `0` is success and the source remains unchanged on failure.'],
  ['buffer_pop', '**buffer_pop** moves the last owning `Buffer<U>` or `OwnedString` element out and returns it in `Result<T, Int32>`.'],
  ['buffer_pop_move_into', '**buffer_pop_move_into** moves the last move-safe owning `T` into a caller-owned output slot without returning a large aggregate through the native ABI.'],
  ['buffer_pop_move_into_status', '**buffer_pop_move_into_status** is the status-code form of `buffer_pop_move_into`; `0` is success and `-20` means the buffer is empty.'],
  ['buffer_remove_move', '**buffer_remove_move** moves one owning `Buffer<U>` or `OwnedString` element out at an index and compacts the source buffer.'],
  ['buffer_remove_move_into', '**buffer_remove_move_into** moves one move-safe owning `T` (nested `Buffer`, carrier, `@repr(C)` enum/record, or fixed owning array) into a caller-owned output slot and compacts the source buffer without duplicate ownership.'],
  ['buffer_remove_move_into_status', '**buffer_remove_move_into_status** is the status-code form of `buffer_remove_move_into`; `0` is success and `-20` means an invalid index.'],
  ['buffer_resize_move', '**buffer_resize_move** resizes an owning nested buffer, direct `Buffer<@repr(C) Record>`, or `Buffer<OwnedString>`; growth zero-initializes storage and shrink destroys removed ownership.'],
  ['buffer_resize_move_status', '**buffer_resize_move_status** is the status-code form of `buffer_resize_move`; it returns `0` on success and preserves the existing bounded Buffer status codes.'],
  ['buffer_remove_drop', '**buffer_remove_drop** destroys one copy-safe or `OwnedString` element and compacts later elements; move-only strings use descriptor-aware cleanup.'],
  ['buffer_remove_drop_status', '**buffer_remove_drop_status** is the status-code form of `buffer_remove_drop`; it returns `0` on success and `-20` for an invalid index.'],
  ['buffer_create', '**buffer_create** creates a generic owning `Buffer<T>`. For `@repr(C)` records with owning `Buffer` fields, `T` is moved and the compiler emits field-table cleanup.'],
  ['buffer_clear', '**buffer_clear** sets a copy-safe `Buffer<T>` length to zero without releasing its reserved allocation. Owning nested buffers remain restricted to move-aware resize APIs.'],
  ['buffer_clear_status', '**buffer_clear_status** is the status-code form of `buffer_clear`; it returns `0` on success and preserves the bounded Buffer status codes.'],
  ['buffer_clear_move', '**buffer_clear_move** recursively destroys initialized nested `Buffer<Buffer<U>>`, owning record, or `Buffer<OwnedString>` values and keeps the outer allocation available for reuse.'],
  ['buffer_clear_move_status', '**buffer_clear_move_status** is the status-code form of `buffer_clear_move`; record and `OwnedString` leaves use destructor-aware cleanup.'],
  ['buffer_reserve', '**buffer_reserve** grows a generic `Buffer<T>`. Copy-safe values use byte moves; owning record fields use the generated move/drop layout.'],
  ['buffer_append', '**buffer_append** appends a scalar, copy-safe nominal, owning `@repr(C)` record, or move-only `OwnedString`. Owning values are transferred into the destination.'],
  ['buffer_append_status', '**buffer_append_status** is the status-code form of `buffer_append`; use it when an owning record append must report bounded OOM/overflow codes.'],
  ['Bool', '**Bool** is the boolean type.'],
  ['Int32', '**Int32** is a signed 32-bit integer type.'],
  ['Int64', '**Int64** is a signed 64-bit integer type.'],
  ['UInt32', '**UInt32** is an unsigned 32-bit integer type.'],
  ['Float32', '**Float32** is a 32-bit floating-point type.'],
  ['Float64', '**Float64** is a 64-bit floating-point type.'],
  ['String', '**String** is the UTF-8 text type.'],
  ['OwnedString', '**OwnedString** is a move-only owning UTF-8 value. Constructors return `Result<OwnedString, Int32>` and scope cleanup is automatic.'],
  ['Vec2', '**Vec2** is a two-component vector type.'],
  ['Vec3', '**Vec3** is a three-component vector type.'],
  ['Vec4', '**Vec4** is a four-component vector type.'],
]);

// Offline help is deliberately local so it remains available while the LSP is
// starting or a file is temporarily invalid. Keep the Slovak catalogue next
// to the English one; the setting is evaluated for every request, so changing
// `jadren.documentationLanguage` takes effect without restarting the client.
const SLOVAK_OFFLINE_DOCS = new Map([
  ['fn', '**fn** deklaruje funkciu Jadren. Príklad: `fn add(a: Int32, b: Int32) -> Int32 { ... }`'],
  ['let', '**let** deklaruje lokálnu väzbu s uvedeným alebo odvodeným typom.'],
  ['if', '**if** vyhodnotí podmienku a pri hodnote `true` vykoná svoj blok.'],
  ['else', '**else** poskytuje alternatívnu vetvu výrazu `if`.'],
  ['for', '**for** iteruje cez rozsah alebo kolekciu. Príklad: `for item in items { ... }`'],
  ['while', '**while** opakuje blok, kým je jeho podmienka pravdivá.'],
  ['return', '**return** ukončí aktuálnu funkciu a vráti výslednú hodnotu.'],
  ['module', '**module** deklaruje názov modulu na začiatku zdrojového súboru Jadren.'],
  ['import', '**import** sprístupní verejné deklarácie z iného modulu.'],
  ['struct', '**struct** deklaruje zložený typ s pomenovanými poľami.'],
  ['enum', '**enum** deklaruje typ s pevnou množinou variantov.'],
  ['match', '**match** vyberie vetvu pomocou porovnávania vzorov.'],
  ['print', '**print** zapíše hodnotu na štandardný výstup.'],
  ['Bool', '**Bool** je booleovský typ s hodnotou `true` alebo `false`.'],
  ['Int32', '**Int32** je 32-bitové znamienkové celé číslo.'],
  ['Int64', '**Int64** je 64-bitové znamienkové celé číslo.'],
  ['UInt32', '**UInt32** je 32-bitové neznamienkové celé číslo.'],
  ['Float32', '**Float32** je 32-bitové číslo s pohyblivou desatinnou čiarkou.'],
  ['Float64', '**Float64** je 64-bitové číslo s pohyblivou desatinnou čiarkou.'],
  ['String', '**String** je pohľad na text v UTF-8.'],
  ['OwnedString', '**OwnedString** je vlastniaca hodnota textu v UTF-8 s presunovou sémantikou.'],
  ['Vec2', '**Vec2** je vektor s dvoma zložkami.'],
  ['Vec3', '**Vec3** je vektor s troma zložkami.'],
  ['Vec4', '**Vec4** je vektor so štyrmi zložkami.'],
  ['stdin_read', '**stdin_read** načíta najviac kapacitu caller-owned `write Slice<UInt8>` zo štandardného vstupu.'],
  ['stdout_write', '**stdout_write** zapíše caller-owned `read Slice<UInt8>` na štandardný výstup.'],
  ['stderr_write', '**stderr_write** zapíše caller-owned `read Slice<UInt8>` na chybový výstup.'],
  ['time_now_unix_seconds', '**time_now_unix_seconds** vráti aktuálny UTC Unix čas v sekundách ako `Int64`.'],
  ['time_now_monotonic_ms', '**time_now_monotonic_ms** vráti monotónny čas v milisekundách ako `UInt64`.'],
  ['process_arg_count', '**process_arg_count** vráti počet argumentov procesu vrátane cesty k executable na indexe `0`.'],
  ['process_arg_read', '**process_arg_read** skopíruje jeden argument procesu do caller-owned UTF-8 buffera bez čiastočného zápisu.'],
  ['http_response_status', '**http_response_status** overí HTTP/1.1 status line a vráti trojciferný stav.'],
  ['http_response_status_prefix', '**http_response_status_prefix** číta stav iba z explicitne platného prefixu response buffera.'],
  ['http_response_header', '**http_response_header** skopíruje jednu response hlavičku do caller-owned výstupu.'],
  ['http_response_header_prefix', '**http_response_header_prefix** číta hlavičku z explicitne platného response prefixu.'],
  ['http_response_header_exact', '**http_response_header_exact** vráti `Bool` a samostatnú dĺžku hlavičky, takže platná prázdna hodnota nie je chyba.'],
  ['http_response_body', '**http_response_body** overí `Content-Length` a skopíruje celé telo bez chunked transferu.'],
  ['http_response_body_prefix', '**http_response_body_prefix** číta telo po overení platného response prefixu a kapacity výstupu.'],
  ['http_response_body_exact', '**http_response_body_exact** vráti `Bool` a samostatnú dĺžku tela vrátane `Content-Length: 0`.'],
  ['http_response_body_chunked_exact', '**http_response_body_chunked_exact** dekóduje kompletnú odpoveď `Transfer-Encoding: chunked`, samostatne zapíše dĺžku a pri chybe nevykoná čiastočný zápis.'],
  ['http_request_body_chunked_exact', '**http_request_body_chunked_exact** dekóduje kompletnú požiadavku `Transfer-Encoding: chunked`, samostatne zapíše dĺžku a pri chybe nevykoná čiastočný zápis.'],
  ['http_request_write_header_block', '**http_request_write_header_block** zostaví request s viacerými validovanými CRLF hlavičkami; framing hlavičky a chybné riadky odmietne ešte pred zápisom.'],
  ['bearer_token_matches', '**bearer_token_matches** porovná presný `Bearer ` token v caller-owned scratch priestore bez uloženia auth stavu.'],
  ['cookie_value_matches', '**cookie_value_matches** nájde jeden presný `name=value` pár v `Cookie` hlavičke bez alokácie alebo mutácie session stavu.'],
  ['http_response_write_cookie', '**http_response_write_cookie** overí dynamický názov, hodnotu a atribúty cookie a zostaví jednu `Set-Cookie` response hlavičku bez partial write.'],
  ['http_response_write_cookie_ex', '**http_response_write_cookie_ex** zostaví dynamickú `Set-Cookie` hlavičku s explicitným keep-alive režimom.'],
  ['http_request_chunked_frame_length_prefix', '**http_request_chunked_frame_length_prefix** vráti presnú hranicu prvého kompletného chunked requestu z prijatého prefixu; `0` znamená neplatný alebo neúplný frame a pipelined suffix zostane nedotknutý.'],
  ['http_router_add_exact', '**http_router_add_exact** registruje HTTP route s explicitnou dĺžkou caller-owned response body; nepoužitý koniec slice sa neodošle.'],
  ['http_router_respond_prefix', '**http_router_respond_prefix** spracuje iba explicitný platný prefix requestu pred zápisom route odpovede.'],
  ['http_request_keep_alive', '**http_request_keep_alive** vráti politiku keep-alive platnej HTTP/1.1 požiadavky.'],
  ['rate_limit_allow', '**rate_limit_allow** prijme request v caller-owned fixed-window stave; politika nevytvára zdieľanú mapu ani vlákno.'],
  ['rate_limit_retry_after_ms', '**rate_limit_retry_after_ms** vráti zostávajúce milisekundy okna bez zmeny stavu.'],
  ['app_state_revision', '**app_state_revision** vráti monotónnu lokálnu revíziu úspešných zmien aplikačného stavu.'],
  ['app_data_revision', '**app_data_revision** vráti nepriehľadný equality-only token celého bounded modelu `app_state`, `app_list` a `app_table`; nie je zoraditeľný ani kryptografický.'],
  ['app_data_validate', '**app_data_validate** overí celý bounded model `app_state`, `app_list` a `app_table` bez jeho zmeny.'],
  ['app_data_write_exact_if_revision', '**app_data_write_exact_if_revision** exportuje caller-owned checkpoint iba pri zhode modelovej revízie; stale volanie nemení výstup ani dĺžku.'],
  ['app_data_load_exact_if_revision', '**app_data_load_exact_if_revision** načíta caller-owned checkpoint iba pri zhode modelovej revízie; stale vstup nenahradí novší lokálny model.'],
  ['app_data_save_atomic_if_revision', '**app_data_save_atomic_if_revision** odmietne zastaraný model a iba pri zhode revízie atomicky uloží celý bounded checkpoint.'],
  ['app_data_tx_begin_if_revision', '**app_data_tx_begin_if_revision** otvorí spoločnú modelovú transakciu iba vtedy, keď equality-only token stále zodpovedá očakávanému snapshotu.'],
  ['app_state_read_text_exact', '**app_state_read_text_exact** načíta text do caller-owned buffera a zapíše dĺžku až po úspešnej validácii.'],
  ['app_state_write_json_exact', '**app_state_write_json_exact** vytvorí bounded JSON snapshot do caller-owned bufferov bez čiastočného zápisu.'],
  ['app_state_load_json_exact', '**app_state_load_json_exact** atomicky načíta overený JSON prefix z caller-owned bufferu bez medzisúboru.'],
  ['app_list_read_text_exact', '**app_list_read_text_exact** načíta položku zo zoznamu vrátane platnej prázdnej položky.'],
  ['app_table_read_cell_exact', '**app_table_read_cell_exact** načíta bunku tabuľky bez sentinelovej nejednoznačnosti.'],
  ['ui_theme', '**ui_theme** vyberie aktívnu sémantickú tému: `0` svetlá, `1` tmavá, `2` systémová.'],
  ['ui_theme_color', '**ui_theme_color** vráti farbu z aktívnej témy; vlastnú farbu zapíš ako `0xRRGGBBu32`.'],
  ['ui_button', '**ui_button** vytvorí tlačidlo s efektom hover, stlačenia a focusu.'],
  ['ui_tooltip', '**ui_tooltip** pripojí natívny tooltip pri prechode myšou na interaktívny prvok.'],
  ['ui_menu', '**ui_menu** vytvorí natívne rozbaľovacie menu Windows.'],
  ['ui_image', '**ui_image** vykreslí projektový PNG alebo SVG asset.'],
]);

function offlineDocumentation(label, english, document) {
  if (documentationLanguage(document) === 'sk') {
    return SLOVAK_OFFLINE_DOCS.get(label)
      || `Vstavaná deklarácia Jadren **${label}**. Podrobnosti určuje jej typový podpis.`;
  }
  return english;
}

/*
 * Built-in UI calls have no source declaration the language server can point
 * to. Keep their editor contract here, next to the direct-call runtime API,
 * so completion, hover and signature help work even while `jadren lsp` is
 * starting or the current program has syntax errors.
 */
function uiApi(name, parameters, documentation, returnType = 'Unit') {
  return {
    name,
    parameters: parameters.map(([label, parameterDocumentation]) => ({
      label,
      documentation: parameterDocumentation,
    })),
    documentation,
    returnType,
  };
}

const UI_API = [
  uiApi('time_now_unix_seconds', [], 'Vráti aktuálny UTC Unix čas v sekundách bez hostiteľského mosta.', 'Int64'),
  uiApi('time_now_monotonic_ms', [], 'Vráti monotónny čas v milisekundách na meranie trvania; nie je to dátum.', 'UInt64'),
  uiApi('rate_limit_allow', [
    ['now_ms: UInt64', 'Current monotonic time in milliseconds.'],
    ['window_ms: UInt64', 'Fixed window duration; zero rejects the request.'],
    ['max_requests: UInt64', 'Maximum accepted requests in one window; zero rejects the request.'],
    ['state: write Slice<UInt64>', 'Caller-owned state with at least two elements: window start and accepted count.'],
  ], 'Accepts one request in a caller-owned fixed window without sleeping, allocating, or creating shared state.', 'Bool'),
  uiApi('rate_limit_retry_after_ms', [
    ['now_ms: UInt64', 'Current monotonic time in milliseconds.'],
    ['window_ms: UInt64', 'Fixed window duration.'],
    ['state: read Slice<UInt64>', 'Caller-owned state with the same two-element layout as rate_limit_allow.'],
  ], 'Returns the remaining local rate-limit window without mutating caller state.', 'UInt64'),
  uiApi('process_arg_count', [], 'Returns the native argument count, including the executable path at index 0.', 'UIntSize'),
  uiApi('process_arg_read', [['index: UIntSize', 'Argument index; index 0 is the executable path.'], ['output: write Slice<UInt8>', 'Caller-owned UTF-8 output buffer; no partial write is performed.']], 'Copies one process argument as UTF-8 and returns its byte length, or 0 for an invalid index or insufficient capacity.', 'UIntSize'),
  uiApi('stdin_read', [['output: write Slice<UInt8>', 'Caller-owned buffer filled with bytes read from native standard input.']], 'Reads at most the output capacity from standard input and returns the number of bytes received; `0` means EOF, an empty buffer, or a native read error.', 'UIntSize'),
  uiApi('stdout_write', [['input: read Slice<UInt8>', 'Caller-owned bytes to write to native standard output.']], 'Writes the complete input slice to standard output when the stream accepts it and returns the number of bytes written.', 'UIntSize'),
  uiApi('stderr_write', [['input: read Slice<UInt8>', 'Caller-owned bytes to write to native standard error.']], 'Writes the complete input slice to standard error when the stream accepts it and returns the number of bytes written.', 'UIntSize'),
  uiApi('time_utc_parts', [['timestamp: Int64', 'Unix sekundy, ktoré sa rozložia.'], ['output: write Slice<Int32>', 'Šesť prvkov: rok, mesiac, deň, hodina, minúta, sekunda.']], 'Zapíše UTC kalendárne časti bez použitia lokálneho timezone. Pri malej kapacite vráti false.', 'Bool'),
  uiApi('time_utc_offset_parts', [['timestamp: Int64', 'Unix sekundy v UTC.'], ['offset_minutes: Int32', 'Explicitný pevný posun v minútach, napr. 60 alebo -60.'], ['output: write Slice<Int32>', 'Šesť prvkov: rok, mesiac, deň, hodina, minúta, sekunda.']], 'Aplikuje explicitný pevný offset a zapíše kalendárne časti. Nečíta lokálny timezone ani DST.', 'Bool'),
  uiApi('app_scheduler_clear', [], 'Vymaže 64-slotový caller-driven timer queue bez vlákna.', 'Unit'),
  uiApi('app_scheduler_set', [['task_id: Int32', 'Stabilný identifikátor timeru.'], ['due_unix_seconds: Int64', 'Prvý termín v Unix sekundách.'], ['repeat_seconds: UInt64', '0 pre one-shot, inak interval opakovania.']], 'Vloží alebo nahradí timer. Pri plnom queue alebo neplatnom intervale vráti false.', 'Bool'),
  uiApi('app_scheduler_cancel', [['task_id: Int32', 'Identifikátor timeru na zrušenie.']], 'Zruší timer bez callbacku a vráti, či existoval.', 'Bool'),
  uiApi('app_scheduler_poll', [['now_unix_seconds: Int64', 'Aktuálny čas, voči ktorému sa vyhodnotí due.'], ['output: write Slice<Int32>', 'Caller-owned pole task ID hodnôt.']], 'Zapíše due task ID v stabilnom poradí. Pri malej kapacite nemení queue.', 'UIntSize'),
  uiApi('app_scheduler_count', [], 'Vráti počet aktívnych timerov.', 'UIntSize'),
  uiApi('buffer_create', [['capacity: UIntSize', 'Počiatočný počet rezervovaných prvkov. Typ T sa odvodí z Result<Buffer<T>, Int32>.']], 'Vytvorí generic owning Buffer<T>; @repr(C) record s owning Buffer poľami používa compiler-generated move/drop layout.', 'Result<Buffer<T>, Int32>'),
  uiApi('buffer_clear', [['buffer: write Buffer<T>', 'Copy-safe generic buffer whose logical length becomes zero.']], 'Clears Buffer<T> without releasing reserved capacity. Nested owning buffers use move-aware resize.', 'Bool'),
  uiApi('buffer_clear_status', [['buffer: write Buffer<T>', 'Copy-safe generic buffer whose logical length becomes zero.']], 'Status form of buffer_clear; returns 0 on success or a stable Buffer status code.', 'Int32'),
  uiApi('buffer_clear_move', [['buffer: write Buffer<T>', 'Nested owning buffer, direct Buffer<@repr(C) Record>, or Buffer<OwnedString> with a move-safe cleanup layout.']], 'Destroys initialized nested buffers, record fields, or owned strings, then keeps the outer allocation for reuse.', 'Bool'),
  uiApi('buffer_clear_move_status', [['buffer: write Buffer<T>', 'Nested owning buffer, direct Buffer<@repr(C) Record>, or Buffer<OwnedString> with a move-safe cleanup layout.']], 'Status form of buffer_clear_move; record fields and OwnedString descriptors use destructor-aware cleanup.', 'Int32'),
  uiApi('buffer_reserve', [['buffer: write Buffer<T>', 'Generic owning buffer.'], ['capacity: UIntSize', 'Nová minimálna kapacita.']], 'Zväčší generic Buffer<T>. Pri owning recordoch zachová presun polí bez byte-copy duplicity.', 'Bool'),
  uiApi('buffer_append', [['buffer: write Buffer<T>', 'Destination owning buffer.'], ['value: T', 'Scalar, copy-safe nominal, owning record, or move-only OwnedString.']], 'Transfers an owning value into the destination and emits the matching cleanup glue.', 'Bool'),
  uiApi('buffer_append_status', [['buffer: write Buffer<T>', 'Cieľový owning buffer.'], ['value: T', 'Hodnota, ktorá sa pri úspechu presunie.']], 'Statusová verzia appendu; pri chybe zachová zdrojovú hodnotu podľa move kontraktu.', 'Int32'),
  uiApi('buffer_resize_move', [['buffer: write Buffer<T>', 'Nested owning buffer, direct Buffer<@repr(C) Record>, or Buffer<OwnedString>.'], ['new_length: UIntSize', 'New logical length; growth zero-initializes storage, shrink destroys removed ownership.']], 'Resizes an owning nested, record, or OwnedString buffer with compiler-generated cleanup.', 'Bool'),
  uiApi('buffer_resize_move_status', [['buffer: write Buffer<T>', 'Nested owning buffer, direct Buffer<@repr(C) Record>, or Buffer<OwnedString>.'], ['new_length: UIntSize', 'New logical length; growth zero-initializes storage, shrink destroys removed ownership.']], 'Status form of the owning resize; returns 0 on success and a bounded Buffer status code on failure.', 'Int32'),
  uiApi('buffer_insert_move', [['buffer: write Buffer<T>', 'Owning descriptor-backed destination buffer.'], ['index: UIntSize', 'Insertion position in the range 0..length.'], ['value: T', 'Owning Buffer<U> or OwnedString descriptor moved into the destination.']], 'Moves an owning descriptor into the requested index and shifts later elements right.', 'Bool'),
  uiApi('buffer_insert_move_status', [['buffer: write Buffer<T>', 'Owning descriptor-backed destination buffer.'], ['index: UIntSize', 'Insertion position in the range 0..length.'], ['value: T', 'Owning Buffer<U> or OwnedString descriptor moved into the destination.']], 'Status form of the move insert; returns 0 on success, -20 for an invalid index, and leaves the source unchanged on failure.', 'Int32'),
  uiApi('buffer_insert_move_from', [['buffer: write Buffer<T>', 'Owning destination buffer for a move-safe element.'], ['index: UIntSize', 'Insertion position in the range 0..length.'], ['value: T', 'Move-safe owning nested Buffer, carrier, @repr(C) enum/record, or fixed owning array.']], 'Inserts the caller-owned value at the requested index without returning a large aggregate through the native ABI. On success the source value is zeroed; validation or allocation failure leaves it unchanged.', 'Bool'),
  uiApi('buffer_insert_move_from_status', [['buffer: write Buffer<T>', 'Owning destination buffer for a move-safe element.'], ['index: UIntSize', 'Insertion position in the range 0..length.'], ['value: T', 'Move-safe owning nested Buffer, carrier, @repr(C) enum/record, or fixed owning array.']], 'Status form of buffer_insert_move_from; returns 0 on success and leaves the caller-owned source unchanged on failure.', 'Int32'),
  uiApi('buffer_append_move', [['buffer: write Buffer<T>', 'Owning destination buffer for a move-safe element.'], ['value: T', 'Move-safe owning nested Buffer, carrier, @repr(C) enum/record, or fixed owning array.']], 'Appends the caller-owned value without a large aggregate ABI. On success the source value is zeroed; validation or allocation failure leaves it unchanged.', 'Bool'),
  uiApi('buffer_append_move_status', [['buffer: write Buffer<T>', 'Owning destination buffer for a move-safe element.'], ['value: T', 'Move-safe owning nested Buffer, carrier, @repr(C) enum/record, or fixed owning array.']], 'Status form of buffer_append_move; returns 0 on success and leaves the caller-owned source unchanged on failure.', 'Int32'),
  uiApi('buffer_pop', [['buffer: write Buffer<T>', 'Buffer<Buffer<U>> or Buffer<OwnedString>.']], 'Moves the last owning descriptor out and returns Result<T, Int32>.', 'Result<T, Int32>'),
  uiApi('buffer_pop_move_into', [['buffer: write Buffer<T>', 'Owning buffer whose last move-safe element is moved out.'], ['output: write T', 'Caller-owned output slot prepared for overwrite; an owning Buffer<T> output must be an owned descriptor.']], 'Moves the last move-safe owning element into the output slot and compacts the source buffer without a large aggregate return.', 'Bool'),
  uiApi('buffer_pop_move_into_status', [['buffer: write Buffer<T>', 'Owning buffer whose last move-safe element is moved out.'], ['output: write T', 'Caller-owned output slot prepared for overwrite; an owning Buffer<T> output must be an owned descriptor.']], 'Status form of buffer_pop_move_into; returns 0 on success and -20 when the buffer is empty.', 'Int32'),
  uiApi('buffer_remove_move', [['buffer: write Buffer<T>', 'Buffer<Buffer<U>> or Buffer<OwnedString>.'], ['index: UIntSize', 'Index of the owning descriptor to move.']], 'Moves one owning descriptor out at an index and returns Result<T, Int32>.', 'Result<T, Int32>'),
  uiApi('buffer_remove_move_into', [['buffer: write Buffer<T>', 'Owning buffer whose move-safe element is moved out.'], ['index: UIntSize', 'Index of the element to move.'], ['output: write T', 'Caller-owned output slot prepared for overwrite; an owning Buffer<T> output must be an owned descriptor.']], 'Moves one move-safe owning element into the output slot, compacts the source buffer, and preserves a single owner.', 'Bool'),
  uiApi('buffer_remove_move_into_status', [['buffer: write Buffer<T>', 'Owning buffer whose move-safe element is moved out.'], ['index: UIntSize', 'Index of the element to move.'], ['output: write T', 'Caller-owned output slot prepared for overwrite; an owning Buffer<T> output must be an owned descriptor.']], 'Status form of buffer_remove_move_into; returns 0 on success and -20 for an invalid index.', 'Int32'),
  uiApi('buffer_remove_drop', [['buffer: write Buffer<T>', 'Copy-safe element buffer or Buffer<OwnedString>.'], ['index: UIntSize', 'Index of the element to destroy.']], 'Destroys the selected element and compacts later elements; OwnedString descriptors are moved without duplicating payload ownership.', 'Bool'),
  uiApi('buffer_remove_drop_status', [['buffer: write Buffer<T>', 'Copy-safe element buffer or Buffer<OwnedString>.'], ['index: UIntSize', 'Index of the element to destroy.']], 'Status form of buffer_remove_drop; returns 0 on success and -20 for an invalid index.', 'Int32'),
  uiApi('string_length', [['value: String', 'UTF-8 text view.']], 'Vráti počet UTF-8 bajtov bez alokácie.', 'UIntSize'),
  uiApi('string_equals', [['left: String', 'Prvý UTF-8 text.'], ['right: String', 'Druhý UTF-8 text.']], 'Porovná dva String pohľady bez alokácie.', 'Bool'),
  uiApi('string_builder_append', [['value: String', 'UTF-8 text, ktorý sa má pridať.'], ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer.'], ['offset: UIntSize', 'Aktuálny počet použitých bajtov.']], 'Pridá String do bounded bufferu a vráti nový offset; pri nedostatku kapacity vráti 0.', 'UIntSize'),
  uiApi('string_builder_append_bytes', [['value: read Slice<UInt8>', 'Caller-owned bajty, ktoré sa majú pridať.'], ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer.'], ['offset: UIntSize', 'Aktuálny počet použitých bajtov.']], 'Pridá bajty do bounded bufferu bez alokácie a vráti nový offset.', 'UIntSize'),
  uiApi('string_owned_create', [['capacity: UIntSize', 'Počiatočná kapacita v bajtoch.']], 'Vytvorí move-only OwnedString; výsledkom je Result<OwnedString, Int32>.', 'Result<OwnedString, Int32>'),
  uiApi('string_owned_from', [['value: String', 'UTF-8 text, ktorý sa skopíruje do vlastníctva.']], 'Validuje a skopíruje String do OwnedString.', 'Result<OwnedString, Int32>'),
  uiApi('string_owned_append', [['value: write OwnedString', 'Vlastnený text, ktorý sa zmení.'], ['suffix: String', 'Validovaný UTF-8 text na pridanie.']], 'Pridá text a vráti status; pri chybe nevytvorí partial output.', 'Int32'),
  uiApi('string_owned_length', [['value: read OwnedString', 'Vlastnený UTF-8 text.']], 'Vráti počet UTF-8 bajtov.', 'UIntSize'),
  uiApi('string_owned_copy', [['value: read OwnedString', 'Vlastnený UTF-8 text.'], ['output: write Slice<UInt8>', 'Caller-owned bounded buffer.']], 'Skopíruje text do buffera a vráti počet zapísaných bajtov.', 'UIntSize'),
  uiApi('string_owned_clear', [['value: write OwnedString', 'Vlastnený text, ktorý sa vyprázdni.']], 'Vyprázdni text bez uvoľnenia rezervovanej kapacity.', 'Int32'),
  uiApi('ui_window', [
    ['title: String', 'Text v titulku Windows okna.'],
    ['width: Int32', 'Počiatočná a minimálna šírka okna v pixeloch.'],
    ['height: Int32', 'Počiatočná a minimálna výška okna v pixeloch.'],
    ['background_color: UInt32', 'Farba klientskej plochy, napr. `0xF6F8FCu32`.'],
  ], 'Vytvorí hlavné Windows okno. Volaj ho pred ostatnými UI prvkami.'),
  uiApi('ui_app_begin', [
    ['title: String', 'Window title.'],
    ['width: Int32', 'Initial window width in pixels.'],
    ['height: Int32', 'Initial window height in pixels.'],
    ['background_color: UInt32', 'Window background color, for example `0xF6F8FCu32`.'],
  ], 'Starts the retained UI tree and returns its root node id.', 'Int32'),
  uiApi('ui_app_on_resize', [
    ['event_id: Int32', 'Explicit callback event ID delivered after native layout resize.'],
  ], 'Registers a bounded window-resize callback event.', 'Unit'),
  uiApi('ui_app_on_close', [
    ['event_id: Int32', 'Explicit callback event ID delivered once before native window close.'],
  ], 'Registers a bounded window-close callback event.', 'Unit'),
  uiApi('ui_app_window_width', [], 'Returns the current native client width.', 'Int32'),
  uiApi('ui_app_window_height', [], 'Returns the current native client height.', 'Int32'),
  uiApi('ui_app_panel', [
    ['parent: Int32', 'Parent node id returned by `ui_app_begin` or another panel.'],
    ['width: Int32', 'Panel width in pixels.'], ['height: Int32', 'Panel height in pixels.'],
    ['background_color: UInt32', 'Panel background color.'], ['corner_radius: Int32', 'Panel corner radius.'],
    ['padding: Int32', 'Inner padding in pixels.'], ['gap: Int32', 'Gap between children in pixels.'],
    ['align: Int32', '`0` start, `1` center, `2` end.'], ['stretch: Int32', '`1` stretches the panel on the cross axis.'],
  ], 'Adds a nested retained panel and returns its node id.', 'Int32'),
  uiApi('ui_app_row', [
    ['parent: Int32', 'Parent retained node id.'], ['width: Int32', 'Row width in pixels.'],
    ['height: Int32', 'Row height in pixels.'], ['padding: Int32', 'Inner row padding.'],
    ['gap: Int32', 'Gap between children in pixels.'],
    ['align: Int32', '`0` start, `1` center, `2` end.'],
    ['stretch: Int32', '`1` stretches the row on the cross axis.'],
  ], 'Adds a horizontal retained layout scope and returns its node id.', 'Int32'),
  uiApi('ui_app_top_bar', [
    ['parent: Int32', 'Parent retained node id.'], ['width: Int32', 'Top bar width in pixels.'],
    ['height: Int32', 'Top bar height in pixels.'], ['background_color: UInt32', 'Top bar background color.'],
    ['corner_radius: Int32', 'Top bar corner radius.'], ['padding: Int32', 'Inner top bar padding.'],
    ['gap: Int32', 'Gap between menu and action controls.'],
    ['align: Int32', '`0` start, `1` center, `2` end.'], ['stretch: Int32', '`1` stretches the top bar on the cross axis.'],
  ], 'Adds a horizontal retained top bar with backend-owned background styling and returns its node id.', 'Int32'),
  uiApi('ui_app_menu', [
    ['parent: Int32', 'Parent retained node id, usually a top bar.'], ['label: String', 'Menu trigger label.'],
    ['width: Int32', 'Requested trigger width in pixels.'], ['height: Int32', 'Trigger height in pixels.'],
    ['text_color: UInt32', 'Trigger text color.'], ['background_color: UInt32', 'Trigger background color.'],
    ['corner_radius: Int32', 'Trigger corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a retained menu trigger. Declare `ui_app_menu_item` calls immediately after it; the native popup opens on click.', 'Int32'),
  uiApi('ui_app_menu_item', [
    ['menu: Int32', 'Node id returned by `ui_app_menu`.'], ['label: String', 'Popup item label.'],
    ['event_id: Int32', 'Application event id sent to `jadren_ui_on_click` after selection.'],
  ], 'Adds one bounded item to a retained popup menu and returns false when the menu is invalid or full.', 'Bool'),
  uiApi('ui_app_label', [
    ['parent: Int32', 'Parent panel node id.'], ['text: String', 'Displayed text.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Text color.'], ['background_color: UInt32', 'Label background color.'],
    ['corner_radius: Int32', 'Label corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a label to a retained panel and returns its node id.', 'Int32'),
  uiApi('ui_app_status', [
    ['parent: Int32', 'Parent retained node id.'], ['text: String', 'Initial status text.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Status text color.'], ['background_color: UInt32', 'Status background color.'],
    ['corner_radius: Int32', 'Status corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a retained status label updated by `ui_set_status` and returns its node id.', 'Int32'),
  uiApi('ui_app_button', [
    ['parent: Int32', 'Parent panel node id.'], ['label: String', 'Button label.'],
    ['event_id: Int32', 'Application event id sent to `jadren_ui_on_click`.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Button text color.'], ['background_color: UInt32', 'Button background color.'],
    ['corner_radius: Int32', 'Button corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds an event button to a retained panel and returns its node id.', 'Int32'),
  uiApi('ui_dispatch_event', [
    ['event_id: Int32', 'Application event id previously assigned to a button, input, checkbox, select, list, or table.'],
  ], 'Dispatches the same exported Jadren callback used by native UI events. Unknown event ids return false.', 'Bool'),
  uiApi('ui_app_tooltip', [
    ['node: Int32', 'Node id returned by `ui_app_button` or `ui_app_checkbox`.'],
    ['text: String', 'Hover tooltip text.'], ['width: Int32', 'Maximum tooltip width in pixels.'],
    ['height: Int32', 'Maximum tooltip height in pixels.'], ['text_color: UInt32', 'Tooltip text color.'],
    ['background_color: UInt32', 'Tooltip background color.'], ['corner_radius: Int32', 'Backend corner radius hint.'],
  ], 'Attaches a bounded hover tooltip to a retained event button or checkbox and returns whether it was accepted.', 'Bool'),
  uiApi('ui_app_text_input', [
    ['parent: Int32', 'Parent panel node id.'], ['text: String', 'Initial input text.'],
    ['event_id: Int32', 'Application event id sent when the text changes.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Input text color.'], ['background_color: UInt32', 'Input background color.'],
    ['corner_radius: Int32', 'Input corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a retained single-line text input and returns its node id.', 'Int32'),
  uiApi('ui_app_checkbox', [
    ['parent: Int32', 'Parent panel node id.'], ['label: String', 'Checkbox label.'],
    ['event_id: Int32', 'Application event id sent when the value changes.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Checkbox label color.'], ['background_color: UInt32', 'Checkbox accent color.'],
    ['corner_radius: Int32', 'Checkbox corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
    ['checked: Int32', '`1` starts checked, `0` starts unchecked.'],
  ], 'Adds a retained checkbox to the current panel and returns its node id.', 'Int32'),
  uiApi('ui_app_select', [
    ['parent: Int32', 'Parent panel node id.'],
    ['event_id: Int32', 'Application event id sent when the selection changes.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Select text color.'], ['background_color: UInt32', 'Select background color.'],
    ['corner_radius: Int32', 'Select corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a retained dropdown to the current panel and returns its node id.', 'Int32'),
  uiApi('ui_app_select_option', [
    ['select: Int32', 'Node id returned by `ui_app_select`.'], ['text: String', 'Option label.'],
  ], 'Adds one option to a retained dropdown. Returns false when its bounded option capacity is full.', 'Bool'),
  uiApi('ui_app_select_index', [
    ['select: Int32', 'Node id returned by `ui_app_select`.'],
  ], 'Returns the selected option index, or `-1` when no option is selected.', 'Int32'),
  uiApi('ui_app_select_set_index', [
    ['select: Int32', 'Node id returned by `ui_app_select`.'], ['index: Int32', 'Option index to select.'],
  ], 'Changes the selected option and returns false for an invalid index.', 'Bool'),
  uiApi('ui_app_list', [
    ['parent: Int32', 'Parent panel node id.'],
    ['event_id: Int32', 'Application event id sent when the selection changes.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'List text color.'], ['background_color: UInt32', 'List background color.'],
    ['corner_radius: Int32', 'List corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a retained dynamic list to the current panel and returns its node id.', 'Int32'),
  uiApi('ui_app_list_item', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'], ['text: String', 'Item label.'],
  ], 'Appends one bounded item to a retained list.', 'Bool'),
  uiApi('ui_app_list_read_item', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'],
    ['index: Int32', 'Item index in the retained list.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer for the UTF-8 item text.'],
  ], 'Copies one retained list item into the caller-owned buffer and returns its byte length.', 'UIntSize'),
  uiApi('ui_app_list_clear', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'],
  ], 'Clears all items in a retained list.', 'Bool'),
  uiApi('ui_app_list_count', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'],
  ], 'Returns the current item count of a retained list.', 'Int32'),
  uiApi('ui_app_list_index', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'],
  ], 'Returns the selected item index, or `-1`.', 'Int32'),
  uiApi('ui_app_list_set_index', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'], ['index: Int32', 'Item index to select.'],
  ], 'Changes the selected item and returns false for an invalid index.', 'Bool'),
  uiApi('ui_app_list_bind_app', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'], ['list_id: Int32', 'Bound `app_list` store id.'],
  ], 'Binds a retained list to a bounded app_list store and refreshes it.', 'Bool'),
  uiApi('ui_app_list_refresh', [
    ['list: Int32', 'Node id returned by `ui_app_list`.'],
  ], 'Refreshes a retained list from its bound app_list store.', 'Bool'),
  uiApi('ui_app_table', [
    ['parent: Int32', 'Parent panel node id.'],
    ['event_id: Int32', 'Application event id sent when the selected row changes.'],
    ['width: Int32', 'Requested width in pixels.'], ['height: Int32', 'Height in pixels.'],
    ['text_color: UInt32', 'Table text color.'], ['background_color: UInt32', 'Table background color.'],
    ['corner_radius: Int32', 'Table corner radius.'], ['stretch: Int32', '`1` fills the cross axis.'],
  ], 'Adds a retained report table to the current panel and returns its node id.', 'Int32'),
  uiApi('ui_app_table_column', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['column: Int32', 'Column index in `0..7`.'],
    ['title: String', 'Column heading.'], ['width: Int32', 'Column width in pixels.'],
  ], 'Declares or updates one bounded table column.', 'Bool'),
  uiApi('ui_app_table_cell', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['row: Int32', 'Row index in `0..63`.'],
    ['column: Int32', 'Column index.'], ['text: String', 'Cell text.'],
  ], 'Writes one bounded table cell.', 'Bool'),
  uiApi('ui_app_table_read_cell', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['row: Int32', 'Row index.'],
    ['column: Int32', 'Column index.'], ['output: write Slice<UInt8>', 'Caller-owned UTF-8 destination.'],
  ], 'Reads one table cell into a caller-owned buffer.', 'UIntSize'),
  uiApi('ui_app_table_bind_app', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['table_id: Int32', 'Bound `app_table` store id.'],
    ['column_count: Int32', 'Visible column count in `1..8`.'],
  ], 'Binds a retained table to a bounded app_table store.', 'Bool'),
  uiApi('ui_app_table_refresh', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'],
  ], 'Refreshes a retained table from its bound app_table store.', 'Bool'),
  uiApi('ui_app_table_clear', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'],
  ], 'Clears all rows in a retained table.', 'Bool'),
  uiApi('ui_app_table_row_count', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'],
  ], 'Returns the current bounded row count.', 'Int32'),
  uiApi('ui_app_table_selected_row', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'],
  ], 'Returns the selected row or `-1`.', 'Int32'),
  uiApi('ui_app_table_set_selected_row', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['row: Int32', 'Row index or `-1` to clear selection.'],
  ], 'Changes the selected row and returns false for an invalid row.', 'Bool'),
  uiApi('ui_app_table_sort_text', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['column: Int32', 'Text column index.'],
    ['descending: Bool', '`true` sorts descending; `false` sorts ascending.'],
  ], 'Sorts the bound app_table by one text column and refreshes every bound view.', 'Bool'),
  uiApi('ui_app_table_sort_int', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['column: Int32', 'Signed integer column index.'],
    ['descending: Bool', '`true` sorts descending; `false` sorts ascending.'],
  ], 'Sorts the bound app_table by one typed Int column and refreshes every bound view.', 'Bool'),
  uiApi('ui_app_table_sort_uint', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['column: Int32', 'Unsigned integer column index.'],
    ['descending: Bool', '`true` sorts descending; `false` sorts ascending.'],
  ], 'Sorts the bound app_table by one typed UInt column and refreshes every bound view.', 'Bool'),
  uiApi('ui_app_table_sort_float', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['column: Int32', 'Float column index.'],
    ['descending: Bool', '`true` sorts descending; `false` sorts ascending.'],
  ], 'Sorts the bound app_table by one typed Float column and refreshes every bound view.', 'Bool'),
  uiApi('ui_app_table_sort_bool', [
    ['table: Int32', 'Node id returned by `ui_app_table`.'], ['column: Int32', 'Boolean column index.'],
    ['descending: Bool', '`true` sorts descending; `false` sorts ascending.'],
  ], 'Sorts the bound app_table by one typed Bool column and refreshes every bound view.', 'Bool'),
  uiApi('ui_app_table_filter_text', [
    ['table: Int32', 'Source table node.'], ['destination_table: Int32', 'Destination app_table store id.'],
    ['column: Int32', 'Text column index.'], ['query: String', 'Exact UTF-8 value to match.'],
  ], 'Copies exact matches into a destination app_table and refreshes bound views.', 'Bool'),
  uiApi('ui_app_table_filter_text_ex', [
    ['table: Int32', 'Source table node.'], ['destination_table: Int32', 'Destination app_table store id.'],
    ['column: Int32', 'Text column index.'], ['query: String', 'UTF-8 query.'],
    ['mode: Int32', '`0` exact, `1` contains, `2` prefix, `3` suffix; add `4` for ASCII case-insensitive.'],
  ], 'Filters the bound app_table into a destination store with an explicit bounded match mode.', 'Bool'),
  uiApi('ui_app_table_filter_int', [
    ['table: Int32', 'Source table node.'], ['destination_table: Int32', 'Destination app_table store id.'],
    ['column: Int32', 'Signed integer column index.'], ['query: Int64', 'Exact signed value to match.'],
  ], 'Copies exact typed Int matches into a destination app_table and refreshes bound views.', 'Bool'),
  uiApi('ui_app_table_filter_uint', [
    ['table: Int32', 'Source table node.'], ['destination_table: Int32', 'Destination app_table store id.'],
    ['column: Int32', 'Unsigned integer column index.'], ['query: UInt64', 'Exact unsigned value to match.'],
  ], 'Copies exact typed UInt matches into a destination app_table and refreshes bound views.', 'Bool'),
  uiApi('ui_app_table_filter_float', [
    ['table: Int32', 'Source table node.'], ['destination_table: Int32', 'Destination app_table store id.'],
    ['column: Int32', 'Float column index.'], ['query: Float64', 'Exact finite value to match.'],
  ], 'Copies exact typed Float matches into a destination app_table and refreshes bound views.', 'Bool'),
  uiApi('ui_app_table_filter_bool', [
    ['table: Int32', 'Source table node.'], ['destination_table: Int32', 'Destination app_table store id.'],
    ['column: Int32', 'Boolean column index.'], ['query: Bool', 'Exact boolean value to match.'],
  ], 'Copies exact typed Bool matches into a destination app_table and refreshes bound views.', 'Bool'),
  uiApi('ui_app_end', [
    ['node: Int32', 'The most recently opened root or panel node id.'],
  ], 'Closes the current retained scope. Returns false when scopes are closed out of order.', 'Bool'),
  uiApi('ui_app_run', [], 'Validates the retained tree and starts the active platform message loop.', 'Int32'),
  uiApi('ui_app_bind_app_state', [
    ['node: Int32', 'Node id returned by a retained text input, checkbox, dropdown, list, or table.'],
    ['key: String', 'Bounded `app_state` key for the control value.'],
  ], 'Binds a retained value control to `app_state` by node id; buttons, labels, and panels are rejected.', 'Bool'),
  uiApi('ui_app_refresh_app_state', [
    ['node: Int32', 'Node id returned by a retained text input, checkbox, dropdown, list, or table.'],
  ], 'Refreshes a retained value control from its bound `app_state` value by node id.'),
  uiApi('ui_top_bar', [
    ['height: Int32', 'Výška horného panelu v pixeloch.'],
    ['background_color: UInt32', 'Farba horného panelu, napr. `top_bar`.'],
  ], 'Vytvorí horný panel, ktorý sa pri zmene okna roztiahne na jeho šírku.'),
  uiApi('ui_text', [
    ['text: String', 'Zobrazovaný text alebo znak.'],
    ['x: Int32', 'Vodorovná pozícia v pixeloch.'],
    ['y: Int32', 'Zvislá pozícia v pixeloch.'],
    ['width: Int32', 'Šírka prvku v pixeloch.'],
    ['height: Int32', 'Výška prvku v pixeloch.'],
    ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia prvku.'],
    ['corner_radius: Int32', 'Zaoblenie rohov v pixeloch; `0` znamená bez zaoblenia.'],
  ], 'Zobrazí statický text s absolútnou pozíciou.'),
  uiApi('ui_label', [
    ['text: String', 'Zobrazovaný text.'],
    ['x: Int32', 'Vodorovná pozícia v pixeloch.'],
    ['y: Int32', 'Zvislá pozícia v pixeloch.'],
    ['width: Int32', 'Šírka prvku v pixeloch.'],
    ['height: Int32', 'Výška prvku v pixeloch.'],
    ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia prvku.'],
    ['corner_radius: Int32', 'Zaoblenie rohov v pixeloch.'],
  ], 'Zobrazí popis; jeho šírka sa môže prispôsobiť šírke okna.'),
  uiApi('ui_status', [
    ['text: String', 'Počiatočný text statusu.'],
    ['x: Int32', 'Vodorovná pozícia v pixeloch.'],
    ['y: Int32', 'Zvislá pozícia v pixeloch.'],
    ['width: Int32', 'Šírka prvku v pixeloch.'],
    ['height: Int32', 'Výška prvku v pixeloch.'],
    ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia statusu.'],
    ['corner_radius: Int32', 'Zaoblenie rohov v pixeloch.'],
  ], 'Vytvorí jediný meniteľný status. Jeho text zmení `ui_set_status(...)`.'),
  uiApi('ui_button', [
    ['label: String', 'Text tlačidla.'],
    ['action: String', 'Text, ktorý sa po kliknutí zobrazí v statuse.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka tlačidla.'], ['height: Int32', 'Výška tlačidla.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba tlačidla.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí tlačidlo s hover, pressed a focus efektom.'),
  uiApi('ui_toggle_button', [
    ['label: String', 'Text tlačidla.'], ['action: String', 'Text statusu po kliknutí.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka tlačidla.'], ['height: Int32', 'Výška tlačidla.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba tlačidla.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí prepínacie tlačidlo, ktoré si drží vybraný stav.'),
  uiApi('ui_disabled_button', [
    ['label: String', 'Text tlačidla.'], ['x: Int32', 'Vodorovná pozícia.'],
    ['y: Int32', 'Zvislá pozícia.'], ['width: Int32', 'Šírka tlačidla.'],
    ['height: Int32', 'Výška tlačidla.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba tlačidla.'], ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí neaktívne tlačidlo bez klikateľnej akcie.'),
  uiApi('ui_close_button', [
    ['label: String', 'Text tlačidla.'], ['x: Int32', 'Vodorovná pozícia.'],
    ['y: Int32', 'Zvislá pozícia.'], ['width: Int32', 'Šírka tlačidla.'],
    ['height: Int32', 'Výška tlačidla.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba tlačidla.'], ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí tlačidlo, ktoré zatvorí okno.'),
  uiApi('ui_icon_button', [
    ['icon: String', 'Názov systémovej ikony (`search`, `settings`, `help`, `download`, `save`, `close`, `menu`, `refresh`) alebo Unicode/textová ikona.'], ['action: String', 'Text statusu po kliknutí.'],
    ['right_offset: Int32', 'Odsadenie od pravého okraja okna.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka tlačidla.'], ['height: Int32', 'Výška tlačidla.'],
    ['text_color: UInt32', 'Farba ikony.'], ['background_color: UInt32', 'Farba tlačidla.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí ikonové tlačidlo ukotvené pri pravom okraji.'),
  uiApi('ui_menu_item', [
    ['label: String', 'Text položky menu.'], ['action: String', 'Text statusu po kliknutí.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka položky.'], ['height: Int32', 'Výška položky.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba pozadia.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí jednoduchú klikateľnú položku menu bez rozbalovacieho popupu.'),
  uiApi('ui_menu', [
    ['label: String', 'Text spúšťača menu.'], ['menu_id: Int32', 'Jedinečné ID menu.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka položky.'], ['height: Int32', 'Výška položky.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba pozadia.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí skutočné Win32 rozbalovacie menu. Hneď pod ním deklaruj `ui_menu_option`.'),
  uiApi('ui_menu_option', [
    ['menu_id: Int32', 'ID menu z `ui_menu`.'], ['label: String', 'Text zobrazenej položky.'],
    ['event_id: Int32', 'ID odoslané do `jadren_ui_on_click`.'],
  ], 'Pridá položku do predtým deklarovaného `ui_menu`.'),
  uiApi('ui_event_button', [
    ['label: String', 'Text tlačidla.'], ['event_id: Int32', 'ID odoslané do `jadren_ui_on_click`.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka tlačidla.'], ['height: Int32', 'Výška tlačidla.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba tlačidla.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí tlačidlo, ktoré synchronne zavolá export `jadren_ui_on_click(event_id)`.'),
  uiApi('ui_set_status', [
    ['text: String', 'Nový text existujúceho statusu.'],
  ], 'Zmení text vytvorený cez `ui_status` alebo `ui_layout_status`.'),
  uiApi('ui_set_button_text', [
    ['event_id: Int32', 'ID eventového tlačidla.'], ['text: String', 'Nový text tlačidla.'],
  ], 'Zmení text eventového tlačidla.'),
  uiApi('ui_set_button_enabled', [
    ['event_id: Int32', 'ID eventového tlačidla.'], ['enabled: Int32', '`1` povolí, `0` zakáže tlačidlo.'],
  ], 'Povolí alebo zakáže eventové tlačidlo.'),
  uiApi('ui_state_get', [
    ['slot: Int32', 'Slot stavu v rozsahu `0..31`.'],
  ], 'Vráti hodnotu `Int32` z lokálneho UI state slotu.', 'Int32'),
  uiApi('ui_state_bind', [
    ['event_id: Int32', 'ID checkboxu, prepínača, výberu, zoznamu, tabuľky alebo vstupu.'],
    ['slot: Int32', 'Slot stavu v rozsahu `0..31`.'],
    ['mode: Int32', '`0` checked, `1` index/výber, `2` dĺžka vstupu, `3` počet položiek/riadkov.'],
  ], 'Prepojí natívny ovládací prvok s UI state slotom. Módy `0` a `1` sú obojsmerné; `2` a `3` aktualizujú slot pri udalosti.'),
  uiApi('ui_state_bind_text', [
    ['event_id: Int32', 'ID textového vstupu.'],
    ['slot: Int32', 'Textový slot v rozsahu `0..31`.'],
  ], 'Obojsmerne synchronizuje textový vstup s bounded UTF-16/UTF-8 UI state slotom.'),
  uiApi('ui_state_set', [
    ['slot: Int32', 'Slot stavu v rozsahu `0..31`.'], ['value: Int32', 'Nová celočíselná hodnota.'],
  ], 'Uloží hodnotu do lokálneho UI state slotu.'),
  uiApi('ui_state_text_length', [
    ['slot: Int32', 'Textový slot v rozsahu `0..31`.'],
  ], 'Vráti aktuálnu dĺžku textu v UTF-8 bajtoch.', 'UIntSize'),
  uiApi('ui_state_text_read', [
    ['slot: Int32', 'Textový slot v rozsahu `0..31`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text.'],
  ], 'Skopíruje textový slot do bounded bufferu bez čiastočného zápisu.', 'UIntSize'),
  uiApi('ui_state_text_set', [
    ['slot: Int32', 'Textový slot v rozsahu `0..31`.'],
    ['text: String', 'Nový UTF-8 text.'],
  ], 'Nastaví textový slot a aplikuje ho do naviazaného natívneho vstupu.', 'Bool'),
  uiApi('ui_column', [
    ['x: Int32', 'Vodorovná pozícia koreňového kontajnera.'], ['y: Int32', 'Zvislá pozícia koreňového kontajnera.'],
    ['width: Int32', 'Šírka kontajnera.'], ['height: Int32', 'Výška kontajnera.'],
    ['padding: Int32', 'Vnútorné odsadenie.'], ['gap: Int32', 'Medzera medzi deťmi.'],
    ['align: Int32', '`0` start, `1` center, `2` end.'], ['stretch: Int32', '`1` roztiahne priečnu os pri resize, `0` ponechá šírku.'],
  ], 'Otvorí koreňový responzívny zvislý layout. Uzavri ho `ui_layout_end()`.'),
  uiApi('ui_row', [
    ['width: Int32', 'Šírka riadku.'], ['height: Int32', 'Výška riadku.'],
    ['padding: Int32', 'Vnútorné odsadenie.'], ['gap: Int32', 'Medzera medzi deťmi.'],
    ['align: Int32', '`0` start, `1` center, `2` end.'], ['stretch: Int32', '`1` roztiahne priečnu os pri resize.'],
  ], 'Otvorí vodorovný layout vo vnútri aktívneho columnu alebo panelu.'),
  uiApi('ui_panel', [
    ['width: Int32', 'Šírka panelu.'], ['height: Int32', 'Výška panelu.'],
    ['background_color: UInt32', 'Farba pozadia panelu.'], ['corner_radius: Int32', 'Zaoblenie rohov.'],
    ['padding: Int32', 'Vnútorné odsadenie.'], ['gap: Int32', 'Medzera medzi deťmi.'],
    ['align: Int32', '`0` start, `1` center, `2` end.'], ['stretch: Int32', '`1` roztiahne panel pri resize.'],
  ], 'Otvorí panel s vlastným zvislým layoutom. Uzavri ho `ui_layout_end()`.'),
  uiApi('ui_layout_label', [
    ['text: String', 'Zobrazovaný text.'], ['width: Int32', 'Požadovaná šírka v pixeloch.'],
    ['height: Int32', 'Výška v pixeloch.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'], ['corner_radius: Int32', 'Zaoblenie rohov.'],
    ['stretch: Int32', '`1` vyplní priečnu os aktívneho layoutu.'],
  ], 'Pridá popis do aktívneho layoutového kontajnera.'),
  uiApi('ui_layout_status', [
    ['text: String', 'Počiatočný text statusu.'], ['width: Int32', 'Požadovaná šírka v pixeloch.'],
    ['height: Int32', 'Výška v pixeloch.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'], ['corner_radius: Int32', 'Zaoblenie rohov.'],
    ['stretch: Int32', '`1` vyplní priečnu os aktívneho layoutu.'],
  ], 'Pridá meniteľný status do aktívneho layoutu.'),
  uiApi('ui_layout_event_button', [
    ['label: String', 'Text tlačidla.'], ['event_id: Int32', 'ID odoslané do callbacku.'],
    ['width: Int32', 'Šírka tlačidla.'], ['height: Int32', 'Výška tlačidla.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba tlačidla.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'], ['stretch: Int32', '`1` vyplní priečnu os aktívneho layoutu.'],
  ], 'Pridá eventové tlačidlo do aktívneho layoutu.'),
  uiApi('ui_layout_end', [], 'Uzavrie aktuálny `ui_column`, `ui_row` alebo `ui_panel` layout.'),
  uiApi('ui_checkbox', [
    ['label: String', 'Text checkboxu.'], ['event_id: Int32', 'ID odoslané do callbacku.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka prvku.'], ['height: Int32', 'Výška prvku.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba akcentu.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí checkbox. Jeho stav čítaj cez `ui_checked(event_id)`. '),
  uiApi('ui_switch', [
    ['label: String', 'Text prepínača.'], ['event_id: Int32', 'ID odoslané do callbacku.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka prvku.'], ['height: Int32', 'Výška prvku.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba akcentu.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí prepínač. Jeho stav čítaj cez `ui_checked(event_id)`. '),
  uiApi('ui_checked', [
    ['event_id: Int32', 'ID checkboxu alebo switchu.'],
  ], 'Vráti stav checkboxu alebo switchu: `1` zapnutý, `0` vypnutý.', 'Int32'),
  uiApi('ui_set_checked', [
    ['event_id: Int32', 'ID checkboxu alebo switchu.'], ['checked: Int32', '`1` zapne, `0` vypne prvok.'],
  ], 'Programovo zmení stav checkboxu alebo switchu.'),
  uiApi('ui_checkbox_bind_app_state', [
    ['event_id: Int32', 'ID checkboxu alebo switchu.'], ['key: String', 'Kľúč Bool hodnoty v bounded `app_state`.'],
  ], 'Naviaže checkbox alebo switch na `app_state`; natívne zmeny synchronizuje.', 'Bool'),
  uiApi('ui_checkbox_refresh_app_state', [
    ['event_id: Int32', 'ID checkboxu alebo switchu.'],
  ], 'Načíta Bool stav checkboxu alebo switchu po `app_state_load`.'),
  uiApi('ui_text_input', [
    ['text: String', 'Počiatočný text vstupu.'], ['event_id: Int32', 'ID odoslané pri zmene textu.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Šírka vstupu.'], ['height: Int32', 'Výška vstupu.'],
    ['text_color: UInt32', 'Farba textu.'], ['background_color: UInt32', 'Farba pozadia.'],
    ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí textový vstup. Zmena textu volá Jadren callback.'),
  uiApi('ui_set_input_text', [
    ['event_id: Int32', 'ID textového vstupu.'], ['text: String', 'Nový text vstupu.'],
  ], 'Programovo zmení text textového vstupu.'),
  uiApi('ui_set_input_enabled', [
    ['event_id: Int32', 'ID textového vstupu.'], ['enabled: Int32', '`1` povolí, `0` zakáže vstup.'],
  ], 'Povolí alebo zakáže textový vstup.'),
  uiApi('ui_input_length', [
    ['event_id: Int32', 'ID textového vstupu.'],
  ], 'Vráti aktuálnu dĺžku textu vo vstupnom poli v UTF-8 bajtoch.', 'UIntSize'),
  uiApi('ui_input_read', [
    ['event_id: Int32', 'ID textového vstupu.'], ['output: write Slice<UInt8>', 'Zapisovateľný pohľad na pole, do ktorého sa skopíruje UTF-8 text.'],
  ], 'Skopíruje aktuálny text vstupu do caller-owned poľa alebo bufferu a vráti počet skopírovaných bajtov.', 'UIntSize'),
  uiApi('ui_input_read_exact', [
    ['event_id: Int32', 'ID textového vstupu.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s aspoň jedným prvkom pre dĺžku.'],
  ], 'Vráti true aj pri platnom prázdnom vstupe a zapíše presnú UTF-8 dĺžku; pri chybe nemení výstupy.', 'Bool'),
  uiApi('ui_input_bind_app_state', [
    ['event_id: Int32', 'ID textového vstupu.'], ['key: String', 'Kľúč textovej hodnoty v bounded `app_state`.'],
  ], 'Naviaže vstup na `app_state`; aktuálny text uloží a ďalšie natívne úpravy synchronizuje.'),
  uiApi('ui_input_refresh_app_state', [
    ['event_id: Int32', 'ID textového vstupu.'],
  ], 'Načíta text z napojeného `app_state` kľúča po `app_state_load`; chýbajúci kľúč znamená prázdny vstup.'),
  uiApi('ui_select_bind_app_state', [
    ['event_id: Int32', 'ID dropdownu.'], ['key: String', 'Kľúč Int64 indexu v bounded `app_state`.'],
  ], 'Naviaže dropdown na `app_state`; zmena výberu synchronizuje index.', 'Bool'),
  uiApi('ui_select_refresh_app_state', [
    ['event_id: Int32', 'ID dropdownu.'],
  ], 'Načíta index dropdownu po `app_state_load`; neplatný index zruší výber.'),
  uiApi('ui_list_bind_app_state', [
    ['event_id: Int32', 'ID zoznamu.'], ['key: String', 'Kľúč Int64 indexu v bounded `app_state`.'],
  ], 'Naviaže zoznam na `app_state`; zmena vybratej položky synchronizuje index.', 'Bool'),
  uiApi('ui_list_refresh_app_state', [
    ['event_id: Int32', 'ID zoznamu.'],
  ], 'Načíta index zoznamu po `app_state_load`; neplatný index zruší výber.'),
  uiApi('ui_table_bind_app_state', [
    ['event_id: Int32', 'ID tabuľky.'], ['key: String', 'Kľúč Int64 indexu riadku v bounded `app_state`.'],
  ], 'Naviaže tabuľku na `app_state`; zmena vybratého riadku synchronizuje index.', 'Bool'),
  uiApi('ui_table_refresh_app_state', [
    ['event_id: Int32', 'ID tabuľky.'],
  ], 'Načíta index riadku tabuľky po `app_state_load`; neplatný index zruší výber.'),
  uiApi('app_state_clear', [], 'Vymaže bounded procesovo-lokálny aplikačný stav.'),
  uiApi('app_state_count', [], 'Vráti počet aktuálne uložených stavových záznamov.', 'Int32'),
  uiApi('app_state_revision', [], 'Vráti monotónnu procesovo-lokálnu generáciu úspešných zmien app_state; program ju môže použiť na explicitné obnovenie UI.', 'UInt64'),
  uiApi('app_data_revision', [], 'Vráti nepriehľadný token snapshotu celého bounded modelu app_state, app_list a app_table. Porovnávaj iba rovnosť; token nie je zoraditeľný ani kryptografický.', 'UInt64'),
  uiApi('app_data_validate', [], 'Overí konzistenciu celého bounded modelu state/list/table bez mutácie; použi pred exportom, sieťovým odoslaním alebo commitom.', 'Bool'),
  uiApi('app_state_type_at', [
    ['index: Int32', 'Logical state-entry index.'],
  ], 'Returns the stored value kind: 1 Int, 2 UInt, 3 Bool, 4 Text, 5 Float; 0 for an invalid index.', 'Int32'),
  uiApi('app_state_exists', [
    ['key: String', 'Kľúč aplikačného stavu.'],
  ], 'Overí existenciu kľúča bez zámienky s nulovou alebo false hodnotou getterov.', 'Bool'),
  uiApi('app_state_remove', [
    ['key: String', 'Kľúč záznamu, ktorý sa má odstrániť.'],
  ], 'Odstráni záznam a zachová poradie zvyšných kľúčov.', 'Bool'),
  uiApi('app_state_set_int', [
    ['key: String', 'Bezpečný kľúč bez úvodzoviek, spätného lomítka a control bytes.'],
    ['value: Int64', 'Signed celočíselná hodnota.'],
  ], 'Nastaví alebo nahradí signed hodnotu v aplikačnom stave.', 'Bool'),
  uiApi('app_state_get_int', [
    ['key: String', 'Kľúč aplikačného stavu.'],
  ], 'Vráti signed hodnotu alebo `0`; po načítaní prijme aj kladný unsigned integer.', 'Int64'),
  uiApi('app_state_set_uint', [
    ['key: String', 'Bezpečný kľúč aplikačného stavu.'],
    ['value: UInt64', 'Unsigned celočíselná hodnota.'],
  ], 'Nastaví alebo nahradí unsigned hodnotu v aplikačnom stave.', 'Bool'),
  uiApi('app_state_get_uint', [
    ['key: String', 'Kľúč aplikačného stavu.'],
  ], 'Vráti unsigned hodnotu alebo `0`; po načítaní prijme aj nezáporný signed integer.', 'UInt64'),
  uiApi('app_state_set_float', [
    ['key: String', 'Bezpečný kľúč aplikačného stavu.'],
    ['value: Float64', 'Konečná desatinná hodnota; NaN a nekonečno sa odmietnu.'],
  ], 'Nastaví alebo nahradí Float64 hodnotu; JSON uloženie používa deterministický desatinný zápis.', 'Bool'),
  uiApi('app_state_get_float', [
    ['key: String', 'Kľúč aplikačného stavu.'],
  ], 'Vráti Float64 hodnotu alebo `0.0`; signed a unsigned celočísla sa dajú bezpečne prečítať ako Float64.', 'Float64'),
  uiApi('app_state_set_bool', [
    ['key: String', 'Bezpečný kľúč aplikačného stavu.'],
    ['value: Bool', 'Boolean hodnota.'],
  ], 'Nastaví alebo nahradí boolean hodnotu v aplikačnom stave.', 'Bool'),
  uiApi('app_state_get_bool', [
    ['key: String', 'Kľúč aplikačného stavu.'],
  ], 'Vráti boolean hodnotu alebo `false`.', 'Bool'),
  uiApi('app_state_set_text', [
    ['key: String', 'Bezpečný kľúč aplikačného stavu.'],
    ['value: String', 'UTF-8 text s maximálne 256 bajtmi.'],
  ], 'Nastaví alebo nahradí textovú hodnotu v aplikačnom stave.', 'Bool'),
  uiApi('app_state_set_text_bytes', [
    ['key: String', 'Bezpečný kľúč aplikačného stavu.'],
    ['value: read Slice<UInt8>', 'Caller-owned UTF-8 bajty.'],
    ['value_length: UIntSize', 'Počet platných bajtov; nesmie prekročiť kapacitu slice.'],
  ], 'Nastaví text z explicitného byte prefixu bez vytvorenia dočasného String.', 'Bool'),
  uiApi('app_state_read_text', [
    ['key: String', 'Kľúč textovej hodnoty.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text.'],
  ], 'Skopíruje text do bufferu a vráti počet bajtov; bounded store nepoužíva alokáciu.', 'UIntSize'),
  uiApi('app_state_read_text_exact', [
    ['key: String', 'Kľúč presne uloženej textovej hodnoty.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s aspoň jedným prvkom pre skopírovanú dĺžku.'],
  ], 'Skopíruje aj prázdny text a vráti true; pri chýbajúcom kľúči, inom druhu alebo krátkom výstupe nemení výstupy.', 'Bool'),
  uiApi('app_state_write_json_exact', [
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý bounded JSON snapshot.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s aspoň jedným prvkom pre výslednú dĺžku.'],
  ], 'Zapíše kompletný JSON snapshot až po overení celej kapacity; pri krátkom výstupe nemení buffer ani dĺžku.', 'Bool'),
  uiApi('app_state_load_json_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned JSON buffer.'],
    ['input_length: UIntSize', 'Platný prefix vstupného bufferu.'],
  ], 'Overí celý JSON prefix v dočasnom stave a až potom atomicky nahradí app_state; chybný vstup živý stav nemení.', 'Bool'),
  uiApi('app_state_read_int', [
    ['key: String', 'Kľúč presne uloženej Int64 hodnoty.'],
    ['output: write Slice<Int64>', 'Caller-owned output slice s aspoň jedným prvkom.'],
  ], 'Zapíše output[0] iba pri presnom druhu Int64; pri chybe vráti false bez zmeny outputu.', 'Bool'),
  uiApi('app_state_read_uint', [
    ['key: String', 'Kľúč presne uloženej UInt64 hodnoty.'],
    ['output: write Slice<UInt64>', 'Caller-owned output slice s aspoň jedným prvkom.'],
  ], 'Zapíše output[0] iba pri presnom druhu UInt64; nevykonáva signed konverziu.', 'Bool'),
  uiApi('app_state_read_float', [
    ['key: String', 'Kľúč presne uloženej Float64 hodnoty.'],
    ['output: write Slice<Float64>', 'Caller-owned output slice s aspoň jedným prvkom.'],
  ], 'Zapíše output[0] iba pri presnom druhu Float64 a vráti false pri chýbajúcej alebo inej hodnote.', 'Bool'),
  uiApi('app_state_read_bool', [
    ['key: String', 'Kľúč presne uloženej Bool hodnoty.'],
    ['output: write Slice<Bool>', 'Caller-owned output slice s aspoň jedným prvkom.'],
  ], 'Zapíše output[0] iba pri presnom druhu Bool; platná false sa odlišuje od chyby cez návratový Bool.', 'Bool'),
  uiApi('app_state_read_key', [
    ['key_index: Int32', 'Index kľúča v poradí vloženia.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 kľúč.'],
  ], 'Skopíruje kľúč podľa indexu a vráti počet bajtov.', 'UIntSize'),
  uiApi('app_state_save', [
    ['path: String', 'Cesta k flat JSON súboru.'],
    ], 'Uloží celý aplikačný stav do bounded flat JSON objektu. Pre crash-safe výmenu použi dočasný súbor a `file_replace_atomic`.', 'Bool'),
  uiApi('app_state_save_atomic', [
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny JSON súbor, ktorý sa atomicky nahradí.'],
  ], 'Uloží stav do temporary_path a vykoná atomickú výmenu za target_path.', 'Bool'),
  uiApi('app_state_save_atomic_if_revision', [
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny JSON súbor, ktorý sa atomicky nahradí.'],
    ['expected_revision: UInt64', 'Revízia, ktorú musí mať stav pred uložením.'],
  ], 'Odmietne stale snapshot a iba pri zhodnej revízii vykoná atomickú výmenu.', 'Bool'),
  uiApi('app_state_load', [
    ['path: String', 'Cesta k flat JSON súboru.'],
  ], 'Načíta flat JSON do aplikačného stavu; pri chybe zachová pôvodný stav.', 'Bool'),
  uiApi('app_state_tx_begin', [], 'Začne bounded procesovú transakciu snapshotom aktuálneho aplikačného stavu; vnorený begin odmietne.', 'Bool'),
  uiApi('app_state_tx_commit', [], 'Potvrdí aktívnu stavovú transakciu a zahodí snapshot.', 'Bool'),
  uiApi('app_state_tx_rollback', [], 'Vráti aktívnu stavovú transakciu zo snapshotu a zahodí zmeny.', 'Bool'),
  uiApi('app_data_tx_begin', [], 'Začne spoločný snapshot app_state, všetkých app_list a všetkých app_table modelov.', 'Bool'),
  uiApi('app_data_tx_begin_if_revision', [
    ['expected_revision: UInt64', 'Predtým uložený equality-only token z `app_data_revision()`.'],
  ], 'Začne spoločnú modelovú transakciu iba pri zhodnom tokene; stale snapshot odmietne bez otvorenia transakcie.', 'Bool'),
  uiApi('app_data_tx_commit', [], 'Potvrdí zmeny celého aplikačného modelu naraz.', 'Bool'),
  uiApi('app_data_tx_rollback', [], 'Obnoví stav, zoznamy a tabuľky zo spoločného snapshotu.', 'Bool'),
  uiApi('app_data_save', [
    ['path: String', 'Cesta k checkpoint súboru.'],
  ], 'Uloží stav, všetky bounded zoznamy a tabuľky do jedného length-framed checkpointu.', 'Bool'),
  uiApi('app_data_write_exact', [
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer pre celý checkpoint.'],
    ['length: write Slice<UIntSize>', 'Caller-owned výstup pre presný počet zapísaných bajtov.'],
  ], 'Exportuje celý bounded model do pamäti bez súboru; krátky buffer nespôsobí čiastočný zápis.', 'Bool'),
  uiApi('app_data_write_exact_if_revision', [
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer pre celý checkpoint.'],
    ['length: write Slice<UIntSize>', 'Caller-owned výstup pre presný počet zapísaných bajtov.'],
    ['expected_revision: UInt64', 'Revízia z `app_data_revision()`, ktorú musí mať model pred exportom.'],
  ], 'Odmietne stale model a iba pri zhodnej revízii exportuje celý checkpoint do pamäti.', 'Bool'),
  uiApi('app_data_save_atomic', [
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny checkpoint, ktorý sa atomicky nahradí.'],
  ], 'Uloží celý aplikačný model a vykoná atomickú výmenu checkpointu.', 'Bool'),
  uiApi('app_data_save_atomic_if_revision', [
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny checkpoint, ktorý sa atomicky nahradí.'],
    ['expected_revision: UInt64', 'Revízia z `app_data_revision()`, ktorú musí mať model pred uložením.'],
  ], 'Odmietne stale model a iba pri zhodnej revízii atomicky uloží celý bounded checkpoint.', 'Bool'),
  uiApi('app_data_tx_save_atomic', [
    ['temporary_path: String', 'Dočasný checkpoint v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny checkpoint, ktorý sa atomicky nahradí.'],
  ], 'Uloží aktívnu modelovú transakciu atomicky a pri úspechu ju potvrdí; pri chybe ju vráti späť.', 'Bool'),
  uiApi('app_data_tx_commit_durable', [
    ['temporary_path: String', 'Dočasný checkpoint v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny checkpoint, ktorý sa atomicky nahradí.'],
    ['lock_path: String', 'Samostatný súbor pre neblokujúci cross-process lock.'],
  ], 'Commitne aktívnu transakciu po flush dočasného checkpointu, locku a atomickej výmene; pri chybe po výmene môže zostať nový súbor na disku.', 'Bool'),
  uiApi('app_data_journal_append', [
    ['journal_path: String', 'Append-only journal file.'],
    ['scratch_path: String', 'Caller-owned temporary checkpoint path.'],
  ], 'Appends one complete bounded model checkpoint as a length-framed journal record.', 'Bool'),
  uiApi('app_data_journal_append_durable', [
    ['journal_path: String', 'Append-only journal file.'],
    ['scratch_path: String', 'Caller-owned temporary checkpoint path.'],
    ['lock_path: String', 'Samostatný súbor pre neblokujúci cross-process lock.'],
  ], 'Appendne checkpoint pod samostatným lockom a flushne journal pred uvoľnením locku.', 'Bool'),
  uiApi('app_data_journal_recover', [
    ['journal_path: String', 'Append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
  ], 'Recovers the last complete journal record and loads it; an incomplete tail is ignored.', 'Bool'),
  uiApi('app_data_journal_recover_compact', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for the compacted journal.'],
  ], 'Recovers the last valid record, writes one canonical frame, and atomically replaces the journal.', 'Bool'),
  uiApi('app_data_journal_recover_compact_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for the compacted journal.'],
    ['lock_path: String', 'Separate cross-process lock path held for recovery and replace.'],
  ], 'Recovers and compacts under a cross-process lock, flushes the temporary and promoted journal, and then releases the lock.', 'Bool'),
  uiApi('app_data_journal_compact_if_over_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for compaction.'],
    ['lock_path: String', 'Separate cross-process lock path held for the operation.'],
    ['max_bytes: UIntSize', 'Caller-selected size threshold; journals at or below it are unchanged.'],
  ], 'Compacts only when the journal exceeds the threshold, holding the lock and flushing the atomic rotation; no implicit retry or worker is created.', 'Bool'),
  uiApi('app_data_journal_compact_if_needed_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for compaction.'],
    ['lock_path: String', 'Separate cross-process lock path held for the full maintenance pass.'],
    ['max_bytes: UIntSize', 'Caller-selected byte threshold; must be nonzero.'],
    ['max_frames: UIntSize', 'Caller-selected complete-frame threshold; must be nonzero.'],
  ], 'Checks both limits and compacts under one lock only when either is exceeded; otherwise it is a successful no-op with no worker or implicit retry.', 'Bool'),
  uiApi('app_data_journal_compact_if_frames_over_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for compaction.'],
    ['lock_path: String', 'Separate cross-process lock path held for the operation.'],
    ['max_frames: UIntSize', 'Caller-selected maximum number of complete valid frames.'],
  ], 'Compacts only when the journal contains more valid complete frames than the threshold; it holds the lock and flushes the atomic rotation.', 'Bool'),
  uiApi('app_data_journal_retain_last_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for retention.'],
    ['lock_path: String', 'Separate cross-process lock path held for the operation.'],
    ['max_frames: UIntSize', 'Caller-selected number of complete valid frames to retain.'],
  ], 'Retains the last complete valid frames under a cross-process lock, flushing the temporary file and atomically replacing the journal.', 'Bool'),
  uiApi('app_data_journal_recover_frame_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['lock_path: String', 'Separate cross-process lock path held for replay.'],
    ['frame_index: UIntSize', 'Zero-based index among complete valid frames.'],
  ], 'Replays one selected complete valid frame under a cross-process lock and loads it transactionally.', 'Bool'),
  uiApi('app_data_journal_count_frames_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for counting.'],
  ], 'Counts complete valid frames under a cross-process lock; the result defines the zero-based replay range.', 'UIntSize'),
  uiApi('app_data_journal_stats_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for the snapshot.'],
    ['output: write Slice<UIntSize>', 'Caller-owned two-element output: frame count, then byte size.'],
  ], 'Writes complete-frame count and journal byte size from one lock-held snapshot.', 'Bool'),
  uiApi('app_data_journal_maintenance_plan_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for the plan snapshot.'],
    ['max_bytes: UIntSize', 'Caller-selected byte threshold; must be nonzero.'],
    ['max_frames: UIntSize', 'Caller-selected complete-frame threshold; must be nonzero.'],
    ['output: write Slice<UIntSize>', 'Three slots: action (0 none, 1 canonical, 2 retain), frame count, then byte size.'],
  ], 'Returns an advisory maintenance action and consistent count/size snapshot without mutating the journal.', 'Bool'),
  uiApi('app_data_journal_maintenance_retry_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['scratch_path: String', 'Caller-owned recovery checkpoint path.'],
    ['temporary_path: String', 'Distinct temporary path used for compaction.'],
    ['lock_path: String', 'Separate cross-process lock path retried for the full maintenance pass.'],
    ['max_bytes: UIntSize', 'Caller-selected byte threshold; must be nonzero.'],
    ['max_frames: UIntSize', 'Caller-selected complete-frame threshold; must be nonzero.'],
    ['max_attempts: UIntSize', 'Total bounded maintenance attempts; zero is rejected.'],
    ['retry_delay_ms: UIntSize', 'Optional bounded sleep between failed attempts.'],
  ], 'Retries the combined durable maintenance boundary a caller-selected number of times; no background worker or unbounded loop is created.', 'Bool'),
  uiApi('app_data_journal_frame_length_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for reading.'],
    ['frame_index: UIntSize', 'Zero-based index among complete valid frames.'],
  ], 'Returns one complete valid frame payload length without copying it or changing the live model.', 'UIntSize'),
  uiApi('app_data_journal_frame_span_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for the snapshot.'],
    ['frame_index: UIntSize', 'Zero-based index among complete valid frames.'],
    ['output: write Slice<UIntSize>', 'Three slots: frame start offset, total frame bytes, then payload bytes.'],
  ], 'Returns frame offsets and lengths from one lock-held checksum-valid snapshot without copying the payload.', 'Bool'),
  uiApi('app_data_journal_build_index_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['index_path: String', 'Persistent index path atomically replaced on success.'],
    ['temporary_path: String', 'Distinct temporary path used before promotion.'],
    ['lock_path: String', 'Separate cross-process lock held for the complete build.'],
  ], 'Builds a deterministic frame-span index for one journal snapshot and flushes it before promotion.', 'Bool'),
  uiApi('app_data_journal_index_lookup_durable', [
    ['journal_path: String', 'Journal used to validate the index snapshot.'],
    ['index_path: String', 'Persistent frame-span index produced by the build API.'],
    ['lock_path: String', 'Separate cross-process lock held for validation and lookup.'],
    ['frame_index: UIntSize', 'Zero-based frame index.'],
    ['output: write Slice<UIntSize>', 'Three slots: frame offset, total bytes, then payload bytes.'],
  ], 'Looks up one indexed frame span and rejects stale or malformed index data without partial output.', 'Bool'),
  uiApi('app_data_journal_index_export_csv_durable', [
    ['journal_path: String', 'Journal used to validate the index snapshot.'],
    ['index_path: String', 'Persistent frame-span index produced by the build API.'],
    ['lock_path: String', 'Separate cross-process lock held for validation and export.'],
    ['output: write Slice<UInt8>', 'Caller-owned CSV output buffer.'],
  ], 'Exports all indexed spans as numeric CSV after stale-index validation; returns the exact byte length or zero.', 'UIntSize'),
  uiApi('app_data_journal_index_export_csv_file_durable', [
    ['journal_path: String', 'Journal used to validate the index snapshot.'],
    ['index_path: String', 'Persistent frame-span index produced by the build API.'],
    ['lock_path: String', 'Separate cross-process lock held for validation and export.'],
    ['output_path: String', 'Final CSV path atomically replaced after a successful export.'],
    ['temporary_path: String', 'Distinct temporary path used before promotion.'],
  ], 'Streams all indexed spans to a flushed CSV file and atomically promotes it only after validation.', 'Bool'),
  uiApi('app_data_journal_index_range_durable', [
    ['journal_path: String', 'Journal used to validate the index snapshot.'],
    ['index_path: String', 'Persistent frame-span index produced by the build API.'],
    ['lock_path: String', 'Separate cross-process lock held for validation and lookup.'],
    ['start_frame: UIntSize', 'Zero-based first frame in the requested page.'],
    ['max_frames: UIntSize', 'Maximum number of spans to return.'],
    ['output: write Slice<UIntSize>', 'Three UIntSize slots per frame: offset, total bytes, payload bytes.'],
  ], 'Returns a stale-safe page of indexed spans and the number of frames written; short output leaves it unchanged.', 'UIntSize'),
  uiApi('app_data_journal_index_read_page_durable', [
    ['journal_path: String', 'Journal used to validate the index snapshot.'],
    ['index_path: String', 'Persistent frame-span index produced by the build API.'],
    ['lock_path: String', 'Separate cross-process lock held for validation and payload read.'],
    ['start_frame: UIntSize', 'Zero-based first frame in the requested page.'],
    ['max_frames: UIntSize', 'Maximum number of complete payloads to return.'],
    ['output: write Slice<UInt8>', 'Caller-owned contiguous payload bytes for all returned frames.'],
    ['metadata: write Slice<UIntSize>', 'Four UIntSize slots per frame: journal offset, total bytes, payload bytes, output offset.'],
  ], 'Reads a stale-safe payload page in one lock-held snapshot; returns the frame count and leaves both outputs unchanged on validation or capacity failure.', 'UIntSize'),
  uiApi('app_data_journal_read_frame_exact_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for reading.'],
    ['frame_index: UIntSize', 'Zero-based index among complete valid frames.'],
    ['output: write Slice<UInt8>', 'Caller-owned output buffer; no partial write is performed.'],
    ['length: write Slice<UIntSize>', 'Caller-owned one-element length output.'],
  ], 'Reads one complete valid frame under a cross-process lock without changing the live model.', 'Bool'),
  uiApi('app_data_journal_read_latest_frame_exact_durable', [
    ['journal_path: String', 'Existing append-only journal file.'],
    ['lock_path: String', 'Separate cross-process lock path held for discovery and reading.'],
    ['output: write Slice<UInt8>', 'Caller-owned output buffer; no partial write is performed.'],
    ['length: write Slice<UIntSize>', 'Caller-owned one-element length output.'],
  ], 'Reads the newest complete valid frame under one cross-process lock without changing the live model.', 'Bool'),
  uiApi('app_data_load', [
    ['path: String', 'Cesta k checkpoint súboru.'],
  ], 'Overí a načíta celý bounded model naraz; pri chybe zachová pôvodný model.', 'Bool'),
  uiApi('app_data_load_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned checkpoint bytes.'],
    ['input_length: UIntSize', 'Presná dĺžka platného prefixu, najviac kapacita slice.'],
  ], 'Načíta celý bounded model z pamäti až po úplnej validácii; chybný alebo skrátený prefix živý model nezmení.', 'Bool'),
  uiApi('app_data_load_exact_if_revision', [
    ['input: read Slice<UInt8>', 'Caller-owned checkpoint bytes.'],
    ['input_length: UIntSize', 'Presná dĺžka platného prefixu, najviac kapacita slice.'],
    ['expected_revision: UInt64', 'Revízia z `app_data_revision()`, ktorú musí mať model pred načítaním.'],
  ], 'Odmietne stale model a iba pri zhodnej revízii po úplnej validácii nahradí bounded model z pamäti.', 'Bool'),
  uiApi('app_list_clear', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
  ], 'Vymaže bounded aplikačnú textovú kolekciu.'),
  uiApi('app_list_count', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
  ], 'Vráti počet položiek kolekcie; každá kolekcia má najviac 64 položiek.', 'Int32'),
  uiApi('app_list_push_text', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['value: String', 'UTF-8 text s maximálne 256 bajtmi.'],
  ], 'Pridá text na koniec bounded kolekcie.', 'Bool'),
  uiApi('app_list_push_text_bytes', [
    ['list_id: Int32', 'List ID in the bounded range `0..3`.'],
    ['value: read Slice<UInt8>', 'Caller-owned UTF-8 byte slice.'],
    ['value_length: UIntSize', 'Number of valid bytes; must not exceed the slice capacity.'],
  ], 'Appends an explicit UTF-8 byte prefix without constructing a temporary String.', 'Bool'),
  uiApi('app_list_export_csv', [
    ['list_id: Int32', 'List ID in the bounded range `0..3`.'],
    ['output: write Slice<UInt8>', 'Caller-owned output buffer for one-column CSV rows.'],
  ], 'Exports each list item as one CSV row with standard quoting; returns zero when capacity is insufficient.', 'UIntSize'),
  uiApi('app_list_sort_text', [
    ['list_id: Int32', 'List ID in the bounded range `0..3`.'],
    ['descending: Bool', 'False sorts ascending; true sorts descending.'],
  ], 'Stably sorts list items by UTF-8 bytes.', 'Bool'),
  uiApi('app_list_sort_callback', [
    ['list_id: Int32', 'List ID in the bounded range `0..3`; keep it read-only in the callback.'],
    ['comparator: fn(Int32, Int32, Int32) -> Int32', 'Receives `(list_id, left_item, right_item)`; negative/zero/positive controls stable order.'],
  ], 'Stably sorts a bounded list with a read-only comparator and publishes only after a fingerprint-checked scan.', 'Bool'),
  uiApi('app_list_find_text', [
    ['list_id: Int32', 'List ID in the bounded range `0..3`.'],
    ['query: String', 'Exact item text to find.'],
    ['start_index: Int32', 'First item index to inspect.'],
  ], 'Returns the first exact item index or `-1`.', 'Int32'),
  uiApi('app_list_filter_text', [
    ['source_list_id: Int32', 'Source list ID in the bounded range `0..3`.'],
    ['destination_list_id: Int32', 'Different destination list ID.'],
    ['query: String', 'Exact item text to project.'],
  ], 'Clears the destination and copies exact matches in source order.', 'Bool'),
  uiApi('app_list_filter_text_ex', [
    ['source_list_id: Int32', 'Source list ID in the bounded range `0..3`.'],
    ['destination_list_id: Int32', 'Different destination list ID.'],
    ['query: String', 'Text used by the selected filter mode.'],
    ['mode: Int32', '0 exact, 1 contains, 2 prefix, 3 suffix; add 4 for ASCII case-insensitive.'],
  ], 'Projects matching items into a separate bounded list without changing the source.', 'Bool'),
  uiApi('app_list_filter_text_ex_bytes', [
    ['source_list_id: Int32', 'Source list ID in the bounded range `0..3`.'],
    ['destination_list_id: Int32', 'Different destination list ID.'],
    ['query: read Slice<UInt8>', 'Caller-owned UTF-8 byte slice used by the filter.'],
    ['query_length: UIntSize', 'Number of valid query bytes; must not exceed the slice capacity.'],
    ['mode: Int32', '0 exact, 1 contains, 2 prefix, 3 suffix; add 4 for ASCII case-insensitive.'],
  ], 'Projects matching items from an explicit UTF-8 byte prefix without constructing a temporary String.', 'Bool'),
  uiApi('app_list_filter_callback', [
    ['source_list_id: Int32', 'Source list ID in the bounded range `0..3`; keep it read-only.'],
    ['destination_list_id: Int32', 'Different destination list ID.'],
    ['predicate: fn(Int32, Int32) -> Bool', 'Receives `(source_list_id, source_item_index)` and returns whether the item is copied.'],
  ], 'Projects items selected by a bounded read-only callback and publishes the destination only after a fingerprint-checked scan.', 'Bool'),
  uiApi('app_list_page', [
    ['source_list_id: Int32', 'Source list ID in the bounded range `0..3`.'],
    ['destination_list_id: Int32', 'Different destination list ID.'],
    ['start_index: Int32', 'First item index; may equal the source count for an empty final page.'],
    ['page_size: Int32', 'Requested number in `0..64`; the final page may be shorter.'],
  ], 'Projects one bounded list page in source order and publishes the destination only after the copy completes.', 'Bool'),
  uiApi('app_list_read_text', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['item_index: Int32', 'Index položky.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text.'],
  ], 'Skopíruje položku do bufferu a vráti počet bajtov.', 'UIntSize'),
  uiApi('app_list_read_text_exact', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['item_index: Int32', 'Index položky.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s aspoň jedným prvkom pre skopírovanú dĺžku.'],
  ], 'Skopíruje aj prázdnu položku a vráti true; pri chýbajúcej položke alebo krátkom výstupe nemení výstupy.', 'Bool'),
  uiApi('app_list_set_text', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['item_index: Int32', 'Index existujúcej položky.'],
    ['value: String', 'Nový UTF-8 text s maximálne 256 bajtmi.'],
  ], 'Nahradí text existujúcej položky.', 'Bool'),
  uiApi('app_list_set_text_bytes', [
    ['list_id: Int32', 'List ID in the bounded range `0..3`.'],
    ['item_index: Int32', 'Existing item index.'],
    ['value: read Slice<UInt8>', 'Caller-owned UTF-8 byte slice.'],
    ['value_length: UIntSize', 'Number of valid bytes; must not exceed the slice capacity.'],
  ], 'Replaces one list item from an explicit UTF-8 byte prefix without a temporary String.', 'Bool'),
  uiApi('app_list_remove', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['item_index: Int32', 'Index existujúcej položky.'],
  ], 'Odstráni položku a posunie nasledujúce položky doľava.', 'Bool'),
  uiApi('app_list_save', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['path: String', 'Cesta k JSON poli textov.'],
  ], 'Uloží bounded kolekciu ako JSON pole; nepoužíva skrytú alokáciu.', 'Bool'),
  uiApi('app_list_save_atomic', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny JSON súbor, ktorý sa atomicky nahradí.'],
  ], 'Zapíše kolekciu do temporary_path a až potom atomicky nahradí target_path.', 'Bool'),
  uiApi('app_list_load', [
    ['list_id: Int32', 'ID kolekcie v rozsahu `0..3`.'],
    ['path: String', 'Cesta k JSON poľu textov.'],
  ], 'Načíta JSON pole transakčne; pri chybe ponechá pôvodnú kolekciu.', 'Bool'),
  uiApi('app_table_clear', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
  ], 'Vymaže bounded aplikačnú tabuľku; model má 64 riadkov a 8 textových stĺpcov.'),
  uiApi('app_table_row_count', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
  ], 'Vráti aktuálny počet riadkov tabuľky.', 'Int32'),
  uiApi('app_table_tx_begin', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
  ], 'Začne bounded snapshot transakciu pre jednu tabuľku; vnorený begin odmietne.', 'Bool'),
  uiApi('app_table_tx_commit', [], 'Potvrdí aktívne zmeny tabuľky a zahodí snapshot.', 'Bool'),
  uiApi('app_table_tx_rollback', [], 'Obnoví riadky aj schému tabuľky zo snapshotu a zahodí zmeny.', 'Bool'),
  uiApi('app_table_tx_begin_all', [], 'Začne bounded snapshot transakciu nad všetkými štyrmi tabuľkami.', 'Bool'),
  uiApi('app_table_tx_commit_all', [], 'Potvrdí zmeny všetkých tabuliek naraz a zahodí spoločný snapshot.', 'Bool'),
  uiApi('app_table_tx_rollback_all', [], 'Obnoví všetky tabuľky zo spoločného snapshotu a zahodí zmeny.', 'Bool'),
  uiApi('app_table_migration_begin', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['expected_version: Int32', 'Verzia, ktorú migrácia očakáva pred zmenami.'],
    ['target_version: Int32', 'Nová nezáporná verzia po úspešnom commite.'],
  ], 'Začne bounded plán migrácie jednej tabuľky; operácie sa aplikujú atomicky pri commite.', 'Bool'),
  uiApi('app_table_migration_rename_column', [
    ['from: String', 'Existujúci ASCII názov stĺpca.'],
    ['to: String', 'Nový jedinečný ASCII názov stĺpca.'],
  ], 'Pridá do aktuálnej migrácie operáciu premenovania stĺpca.', 'Bool'),
  uiApi('app_table_migration_set_column_type', [
    ['name: String', 'ASCII názov existujúceho stĺpca.'],
    ['kind: Int32', 'Typ `0=text`, `1=int`, `2=uint`, `3=bool`, `4=float`.'],
  ], 'Pridá do aktuálnej migrácie kontrolovanú zmenu typu stĺpca.', 'Bool'),
  uiApi('app_table_migration_commit', [], 'Aplikuje najviac 16 queued operácií a zvýši verziu; pri chybe obnoví snapshot.', 'Bool'),
  uiApi('app_table_migration_rollback', [], 'Zahodí aktuálny plán migrácie a obnoví snapshot tabuľky.', 'Bool'),
  uiApi('app_table_set_column_type', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Index stĺpca v rozsahu `0..7`.'],
    ['kind: Int32', 'Typ `0=text`, `1=int`, `2=uint`, `3=bool`, `4=float`.'],
  ], 'Nastaví bounded runtime schema stĺpca; existujúce hodnoty musia typ vyhovovať.', 'Bool'),
  uiApi('app_table_set_column_name', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Index stĺpca v rozsahu `0..7`.'],
    ['name: String', 'ASCII identifikátor poľa, napríklad `task_id`; prázdny text názov vymaže.'],
  ], 'Pomenuje typed-record stĺpec; názvy musia byť jedinečné v tabuľke.', 'Bool'),
  uiApi('app_table_read_column_name', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Index stĺpca v rozsahu `0..7`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre názov poľa.'],
  ], 'Skopíruje názov stĺpca do bufferu a vráti počet bajtov.', 'UIntSize'),
  uiApi('app_table_read_column_name_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Index stĺpca v rozsahu `0..7`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre názov poľa.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s dĺžkou skopírovaného názvu.'],
  ], 'Skopíruje aj prázdny názov a vráti true; pri chybe nemení výstupy.', 'Bool'),
  uiApi('app_table_find_column', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['name: String', 'Presný názov typed-record poľa.'],
  ], 'Vráti index pomenovaného stĺpca alebo `-1`, ak názov neexistuje.', 'Int32'),
  uiApi('app_table_schema_version', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
  ], 'Vráti aplikačnú verziu full-schema tabuľky; pri neplatnom ID vráti `-1`.', 'Int32'),
  uiApi('app_table_set_schema_version', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['version: Int32', 'Nezáporná verzia schémy riadená aplikáciou.'],
  ], 'Nastaví verziu schémy, ktorá sa uloží do full-schema JSON.', 'Bool'),
  uiApi('app_table_set_named_cell', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Presný názov typed-record poľa.'],
    ['value: String', 'UTF-8 text bunky.'],
  ], 'Zapíše bunku podľa názvu poľa bez ručného hľadania indexu.', 'Bool'),
  uiApi('app_table_read_named_cell', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Presný názov typed-record poľa.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre hodnotu bunky.'],
  ], 'Prečíta bunku podľa názvu poľa a vráti počet bajtov.', 'UIntSize'),
  uiApi('app_table_read_named_cell_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Presný názov typed-record poľa.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre hodnotu bunky.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s dĺžkou skopírovanej bunky.'],
  ], 'Prečíta aj prázdnu bunku podľa názvu a pri chybe nemení výstupy.', 'Bool'),
  uiApi('app_table_set_named_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `int`.'],
    ['value: Int64', 'Signed hodnota.'],
  ], 'Zapíše typed signed hodnotu podľa názvu poľa.', 'Bool'),
  uiApi('app_table_set_named_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `uint`.'],
    ['value: UInt64', 'Unsigned hodnota.'],
  ], 'Zapíše typed unsigned hodnotu podľa názvu poľa.', 'Bool'),
  uiApi('app_table_set_named_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `float`.'],
    ['value: Float64', 'Konečná desatinná hodnota.'],
  ], 'Zapíše typed Float64 hodnotu podľa názvu poľa; NaN a nekonečno odmietne.', 'Bool'),
  uiApi('app_table_set_named_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `bool`.'],
    ['value: Bool', 'Boolean hodnota.'],
  ], 'Zapíše typed bool hodnotu podľa názvu poľa.', 'Bool'),
  uiApi('app_table_read_named_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `int`.'],
  ], 'Prečíta signed hodnotu podľa názvu poľa.', 'Int64'),
  uiApi('app_table_read_named_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `uint`.'],
  ], 'Prečíta unsigned hodnotu podľa názvu poľa.', 'UInt64'),
  uiApi('app_table_read_named_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `float`.'],
  ], 'Prečíta Float64 hodnotu podľa názvu poľa alebo vráti `0.0` pri chybe.', 'Float64'),
  uiApi('app_table_read_named_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `bool`.'],
  ], 'Prečíta bool hodnotu podľa názvu poľa.', 'Bool'),
  uiApi('app_table_read_named_int_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `int`.'],
    ['output: write Slice<Int64>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta signed hodnotu podľa názvu; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_read_named_uint_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `uint`.'],
    ['output: write Slice<UInt64>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta unsigned hodnotu podľa názvu; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_read_named_float_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `float`.'],
    ['output: write Slice<Float64>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta Float64 hodnotu podľa názvu; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_read_named_bool_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_name: String', 'Názov poľa typu `bool`.'],
    ['output: write Slice<Bool>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta bool hodnotu podľa názvu; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_column_type', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Index stĺpca v rozsahu `0..7`.'],
  ], 'Vráti typ stĺpca `0..4`, pri neplatnom indexe `-1`.', 'Int32'),
  uiApi('app_table_validate', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
  ], 'Overí všetky neprázdne bunky podľa runtime schema tabuľky.', 'Bool'),
  uiApi('app_table_append_row', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
  ], 'Pridá prázdny riadok do bounded tabuľky.', 'Bool'),
  uiApi('app_table_remove_row', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
  ], 'Odstráni riadok a posunie nasledujúce riadky doľava.', 'Bool'),
  uiApi('app_table_set_cell', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['value: String', 'UTF-8 text s maximálne 256 bajtmi.'],
  ], 'Nastaví text jednej bunky bez UI alebo skrytej alokácie.', 'Bool'),
  uiApi('app_table_set_cell_bytes', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['value: read Slice<UInt8>', 'Caller-owned UTF-8 bytes s maximálne 256 bajtmi.'],
  ], 'Nastaví text bunky priamo z caller-owned bufferu bez medzikópie do String.', 'Bool'),
  uiApi('app_table_set_cell_bytes_ex', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['value: read Slice<UInt8>', 'Caller-owned UTF-8 buffer s kapacitou maximálne 256 bajtov.'],
    ['value_length: UIntSize', 'Počet platných bajtov; nesmie prekročiť kapacitu slice.'],
  ], 'Nastaví iba platný prefix bufferu, takže UI input nemusí kopírovať nulový zvyšok.', 'Bool'),
  uiApi('app_table_set_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `int` v rozsahu `0..7`.'],
    ['value: Int64', 'Signed hodnota, ktorú runtime deterministicky formátuje.'],
  ], 'Zapíše typed signed bunku bez ručného prevodu na text.', 'Bool'),
  uiApi('app_table_set_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `uint` v rozsahu `0..7`.'],
    ['value: UInt64', 'Unsigned hodnota, ktorú runtime deterministicky formátuje.'],
  ], 'Zapíše typed unsigned bunku bez ručného prevodu na text.', 'Bool'),
  uiApi('app_table_set_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `float` v rozsahu `0..7`.'],
    ['value: Float64', 'Konečná desatinná hodnota, ktorú runtime deterministicky formátuje.'],
  ], 'Zapíše typed Float64 bunku bez ručného prevodu na text; NaN a nekonečno odmietne.', 'Bool'),
  uiApi('app_table_set_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `bool` v rozsahu `0..7`.'],
    ['value: Bool', 'Boolean hodnota zapísaná ako `true` alebo `false`.'],
  ], 'Zapíše typed bool bunku bez ručného prevodu na text.', 'Bool'),
  uiApi('app_table_read_cell', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 bunku.'],
  ], 'Skopíruje bunku do bufferu a vráti počet bajtov.', 'UIntSize'),
  uiApi('app_table_read_cell_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 bunku.'],
    ['length: write Slice<UIntSize>', 'Caller-owned output s dĺžkou skopírovanej bunky.'],
  ], 'Skopíruje aj prázdnu bunku a pri chybe nemení výstupy.', 'Bool'),
  uiApi('app_table_read_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `int` v rozsahu `0..7`.'],
  ], 'Vráti signed hodnotu typed bunky alebo `0` pri neplatnom type/indexe.', 'Int64'),
  uiApi('app_table_read_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `uint` v rozsahu `0..7`.'],
  ], 'Vráti unsigned hodnotu typed bunky alebo `0` pri neplatnom type/indexe.', 'UInt64'),
  uiApi('app_table_read_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `float` v rozsahu `0..7`.'],
  ], 'Vráti Float64 hodnotu typed bunky alebo `0.0` pri neplatnom type/indexe.', 'Float64'),
  uiApi('app_table_read_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `bool` v rozsahu `0..7`.'],
  ], 'Vráti bool hodnotu typed bunky; neplatná alebo prázdna bunka je `false`.', 'Bool'),
  uiApi('app_table_read_int_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `int` v rozsahu `0..7`.'],
    ['output: write Slice<Int64>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta signed hodnotu bez sentinelov; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_read_uint_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `uint` v rozsahu `0..7`.'],
    ['output: write Slice<UInt64>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta unsigned hodnotu bez sentinelov; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_read_float_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `float` v rozsahu `0..7`.'],
    ['output: write Slice<Float64>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta Float64 hodnotu bez sentinelov; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_read_bool_exact', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['row_index: Int32', 'Index existujúceho riadku.'],
    ['column_index: Int32', 'Stĺpec typu `bool` v rozsahu `0..7`.'],
    ['output: write Slice<Bool>', 'Caller-owned výstup pre hodnotu.'],
  ], 'Prečíta bool hodnotu bez sentinelov; pri chybe nemení output a vráti false.', 'Bool'),
  uiApi('app_table_sort_text', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['descending: Bool', 'False zoradí vzostupne, true zostupne.'],
  ], 'Stabilne zoradí riadky podľa UTF-8 bajtov v jednom textovom stĺpci.', 'Bool'),
  uiApi('app_table_find_text', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['query: String', 'Presný text bunky, ktorý sa hľadá.'],
    ['start_row: Int32', 'Prvý riadok, od ktorého sa hľadá.'],
  ], 'Vráti prvý zhodný riadok alebo `-1`, ak sa text nenájde.', 'Int32'),
  uiApi('app_table_find_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Stĺpec typu `int` v rozsahu `0..7`.'],
    ['query: Int64', 'Presná celočíselná hodnota typed bunky.'],
    ['start_row: Int32', 'Prvý riadok, od ktorého sa hľadá.'],
  ], 'Vráti prvý zhodný riadok alebo `-1`; vyžaduje stĺpec typu `int`.', 'Int32'),
  uiApi('app_table_find_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Stĺpec typu `uint` v rozsahu `0..7`.'],
    ['query: UInt64', 'Presná unsigned hodnota typed bunky.'],
    ['start_row: Int32', 'Prvý riadok, od ktorého sa hľadá.'],
  ], 'Vráti prvý zhodný riadok alebo `-1`; vyžaduje stĺpec typu `uint`.', 'Int32'),
  uiApi('app_table_find_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Stĺpec typu `float` v rozsahu `0..7`.'],
    ['query: Float64', 'Presná konečná hodnota typed bunky.'],
    ['start_row: Int32', 'Prvý riadok, od ktorého sa hľadá.'],
  ], 'Vráti prvý zhodný riadok alebo `-1`; vyžaduje stĺpec typu `float`.', 'Int32'),
  uiApi('app_table_find_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Stĺpec typu `bool` v rozsahu `0..7`.'],
    ['query: Bool', 'Presná boolean hodnota typed bunky.'],
    ['start_row: Int32', 'Prvý riadok, od ktorého sa hľadá.'],
  ], 'Vráti prvý zhodný riadok alebo `-1`; vyžaduje stĺpec typu `bool`.', 'Int32'),
  uiApi('app_table_remove_text', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový textový stĺpec v rozsahu `0..7`.'],
    ['key: String', 'Presný neprázdny textový kľúč.'],
  ], 'Odstráni prvý riadok s kľúčom; pri nenájdení vráti false.', 'Bool'),
  uiApi('app_table_remove_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `int` stĺpec v rozsahu `0..7`.'],
    ['key: Int64', 'Presný celočíselný kľúč.'],
  ], 'Odstráni prvý riadok s kľúčom; vyžaduje stĺpec typu `int`.', 'Bool'),
  uiApi('app_table_remove_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `uint` stĺpec v rozsahu `0..7`.'],
    ['key: UInt64', 'Presný unsigned celočíselný kľúč.'],
  ], 'Odstráni prvý riadok s kľúčom; vyžaduje stĺpec typu `uint`.', 'Bool'),
  uiApi('app_table_remove_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `float` stĺpec v rozsahu `0..7`.'],
    ['key: Float64', 'Presný konečný desatinný kľúč.'],
  ], 'Odstráni prvý riadok s kľúčom; vyžaduje stĺpec typu `float`.', 'Bool'),
  uiApi('app_table_remove_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `bool` stĺpec v rozsahu `0..7`.'],
    ['key: Bool', 'Presný boolean kľúč.'],
  ], 'Odstráni prvý riadok s kľúčom; vyžaduje stĺpec typu `bool`.', 'Bool'),
  uiApi('app_table_upsert_text', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový textový stĺpec v rozsahu `0..7`.'],
    ['key: String', 'Neprázdny textový kľúč.'],
  ], 'Vráti prvý riadok s kľúčom alebo vytvorí nový riadok a zapíše kľúč.', 'Int32'),
  uiApi('app_table_upsert_int', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `int` stĺpec v rozsahu `0..7`.'],
    ['key: Int64', 'Celočíselný kľúč.'],
  ], 'Vráti prvý riadok s kľúčom alebo vytvorí nový riadok a zapíše kľúč.', 'Int32'),
  uiApi('app_table_upsert_uint', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `uint` stĺpec v rozsahu `0..7`.'],
    ['key: UInt64', 'Unsigned celočíselný kľúč.'],
  ], 'Vráti prvý riadok s kľúčom alebo vytvorí nový riadok a zapíše kľúč.', 'Int32'),
  uiApi('app_table_upsert_float', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `float` stĺpec v rozsahu `0..7`.'],
    ['key: Float64', 'Konečný desatinný kľúč.'],
  ], 'Vráti prvý riadok s kľúčom alebo vytvorí nový riadok a zapíše kľúč.', 'Int32'),
  uiApi('app_table_upsert_bool', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['column_index: Int32', 'Kľúčový `bool` stĺpec v rozsahu `0..7`.'],
    ['key: Bool', 'Boolean kľúč.'],
  ], 'Vráti prvý riadok s kľúčom alebo vytvorí nový riadok a zapíše kľúč.', 'Int32'),
  uiApi('app_table_index_build', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Column index in the bounded range `0..7`.'],
  ], 'Builds a bounded index using the column declared type; duplicate values keep stable source-row order.', 'Bool'),
  uiApi('app_table_index_build_int', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Declared `Int` column in the bounded range `0..7`.'],
  ], 'Builds a numeric Int index for exact logarithmic lookup.', 'Bool'),
  uiApi('app_table_index_build_uint', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Declared `UInt` column in the bounded range `0..7`.'],
  ], 'Builds a numeric UInt index for exact logarithmic lookup.', 'Bool'),
  uiApi('app_table_index_build_float', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Declared `Float` column in the bounded range `0..7`.'],
  ], 'Builds a numeric Float index for exact logarithmic lookup.', 'Bool'),
  uiApi('app_table_index_build_bool', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Declared `Bool` column in the bounded range `0..7`.'],
  ], 'Builds a Bool index for exact logarithmic lookup.', 'Bool'),
  uiApi('app_table_index_build_pair', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['first_column_index: Int32', 'First declared text column in the bounded range `0..7`.'],
    ['second_column_index: Int32', 'Second distinct declared text column in the bounded range `0..7`.'],
  ], 'Builds a deterministic lexicographic two-key text index; mutations invalidate it by fingerprint.', 'Bool'),
  uiApi('app_table_index_clear', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
  ], 'Clears the table index without changing rows or schema.', 'Bool'),
  uiApi('app_table_index_find_text', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed column in the bounded range `0..7`.'],
    ['query: String', 'Exact cell text to find.'],
  ], 'Returns the first indexed row or `-1`; mutations automatically invalidate the index.', 'Int32'),
  uiApi('app_table_index_find_pair_text', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['first_column_index: Int32', 'First indexed text column in the bounded range `0..7`.'],
    ['second_column_index: Int32', 'Second indexed text column in the bounded range `0..7`.'],
    ['first_query: String', 'Exact value for the first key.'],
    ['second_query: String', 'Exact value for the second key.'],
  ], 'Returns the first row matching both text keys or `-1`; the pair index must be current.', 'Int32'),
  uiApi('app_table_index_find_int', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `Int` column in the bounded range `0..7`.'],
    ['query: Int64', 'Exact signed integer value to find.'],
  ], 'Returns the first indexed row or `-1`; mutations automatically invalidate the index.', 'Int32'),
  uiApi('app_table_index_collect_int_range', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `Int` column in the bounded range `0..7`.'],
    ['lower: Int64', 'Inclusive lower bound.'],
    ['upper: Int64', 'Inclusive upper bound; must be greater than or equal to `lower`.'],
    ['output: write Slice<Int32>', 'Caller-owned row-id storage; capacity must fit every match.'],
  ], 'Writes matching row ids in ascending indexed order and returns the count; a stale index or short output returns zero.', 'UIntSize'),
  uiApi('app_table_index_collect_uint_range', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `UInt` column in the bounded range `0..7`.'],
    ['lower: UInt64', 'Inclusive lower bound.'],
    ['upper: UInt64', 'Inclusive upper bound; must be greater than or equal to `lower`.'],
    ['output: write Slice<Int32>', 'Caller-owned row-id storage; capacity must fit every match.'],
  ], 'Writes matching row ids in ascending indexed order and returns the count; a stale index or short output returns zero.', 'UIntSize'),
  uiApi('app_table_index_collect_float_range', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `Float` column in the bounded range `0..7`.'],
    ['lower: Float64', 'Inclusive finite lower bound.'],
    ['upper: Float64', 'Inclusive finite upper bound; must be greater than or equal to `lower`.'],
    ['output: write Slice<Int32>', 'Caller-owned row-id storage; capacity must fit every match.'],
  ], 'Writes matching row ids in ascending indexed order and returns the count; NaN, stale index, or short output returns zero.', 'UIntSize'),
  uiApi('app_table_index_find_uint', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `UInt` column in the bounded range `0..7`.'],
    ['query: UInt64', 'Exact unsigned integer value to find.'],
  ], 'Returns the first indexed row or `-1`; mutations automatically invalidate the index.', 'Int32'),
  uiApi('app_table_index_find_float', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `Float` column in the bounded range `0..7`.'],
    ['query: Float64', 'Exact finite floating-point value to find.'],
  ], 'Returns the first indexed row or `-1`; mutations automatically invalidate the index.', 'Int32'),
  uiApi('app_table_index_find_bool', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed `Bool` column in the bounded range `0..7`.'],
    ['query: Bool', 'Exact boolean value to find.'],
  ], 'Returns the first indexed row or `-1`; mutations automatically invalidate the index.', 'Int32'),
  uiApi('app_table_index_is_valid', [
    ['table_id: Int32', 'Table ID in the bounded range `0..3`.'],
    ['column_index: Int32', 'Indexed column in the bounded range `0..7`.'],
  ], 'Checks whether the column index still matches the current table data.', 'Bool'),
  uiApi('app_table_filter_text', [
    ['source_table_id: Int32', 'Zdrojová tabuľka v rozsahu `0..3`.'],
    ['destination_table_id: Int32', 'Cieľová tabuľka; musí byť iná než zdrojová.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['query: String', 'Presný text bunky, ktorý sa kopíruje.'],
  ], 'Vyčistí cieľ a skopíruje zhodné riadky v pôvodnom poradí.', 'Bool'),
  uiApi('app_table_filter_text_ex', [
    ['source_table_id: Int32', 'Zdrojová tabuľka v rozsahu `0..3`.'],
    ['destination_table_id: Int32', 'Cieľová tabuľka; musí byť iná než zdrojová.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['query: String', 'Text, ktorý sa má vyhľadať.'],
    ['mode: Int32', '`0` exact, `1` contains, `2` prefix, `3` suffix; `4..7` sú ASCII case-insensitive varianty.'],
  ], 'Rozšírený bounded filter; vyčistí cieľ a zachová poradie zdroja.', 'Bool'),
  uiApi('app_table_filter_text_ex_bytes', [
    ['source_table_id: Int32', 'Zdrojová tabuľka v rozsahu `0..3`.'],
    ['destination_table_id: Int32', 'Cieľová tabuľka; musí byť iná než zdrojová.'],
    ['column_index: Int32', 'Stĺpec v rozsahu `0..7`.'],
    ['query: read Slice<UInt8>', 'Caller-owned UTF-8 buffer s textom filtra.'],
    ['query_length: UIntSize', 'Počet platných bajtov v query; nesmie prekročiť kapacitu slice.'],
    ['mode: Int32', '`0` exact, `1` contains, `2` prefix, `3` suffix; `4..7` sú ASCII case-insensitive varianty.'],
  ], 'Filtruje tabuľku priamo z bounded UTF-8 buffera bez medzikópie do String.', 'Bool'),
  uiApi('app_table_filter_callback', [
    ['source_table_id: Int32', 'Zdrojová tabuľka v rozsahu `0..3`; počas callbacku ju nemen.'],
    ['destination_table_id: Int32', 'Cieľová tabuľka; musí byť iná než zdrojová.'],
    ['predicate: fn(Int32, Int32) -> Bool', 'Predikát dostane `(source_table_id, source_row_index)` a vráti, či sa riadok kopíruje.'],
  ], 'Spustí bounded read-only callback filter v pôvodnom poradí; pri zmene zdroja alebo chybe cieľ ostane nezmenený.', 'Bool'),
  uiApi('app_table_sort_callback', [
    ['table_id: Int32', 'Tabuľka v rozsahu `0..3`; počas callbacku ju nemen.'],
    ['comparator: fn(Int32, Int32, Int32) -> Int32', 'Dostane `(table_id, left_row, right_row)`; záporný/nulový/kladný výsledok určí stabilné poradie.'],
  ], 'Zoradí bounded tabuľku read-only comparatorom a zmenu publikuje až po fingerprint-checked skene.', 'Bool'),
  uiApi('app_table_page', [
    ['source_table_id: Int32', 'Zdrojová tabuľka v rozsahu `0..3`.'],
    ['destination_table_id: Int32', 'Cieľová tabuľka; musí byť iná než zdrojová.'],
    ['start_row: Int32', 'Prvý riadok; môže sa rovnať počtu riadkov pre prázdnu poslednú stránku.'],
    ['page_size: Int32', 'Požadovaný počet v rozsahu `0..64`; posledná stránka môže byť kratšia.'],
  ], 'Premietne jednu bounded stránku riadkov v poradí zdroja, zachová schému a cieľ zverejní až po úplnom skopírovaní.', 'Bool'),
  uiApi('app_table_export_csv', [
    ['table_id: Int32', 'ID tabuľky v rozsahu \`0..3\`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre kompletný CSV export.'],
  ], 'Zapíše hlavičku a všetky aktívne riadky tabuľky ako bounded CSV; pri malej kapacite vráti \`0\` bez partial outputu.', 'UIntSize'),
  uiApi('app_table_import_csv', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['input: read Slice<UInt8>', 'Caller-owned CSV buffer; parser číta iba platný prefix.'],
    ['input_length: UIntSize', 'Počet platných bajtov; nesmie prekročiť kapacitu slice.'],
  ], 'Načíta štandardné bounded CSV transakčne; pri chybe, nezhode stĺpcov alebo neplatných typed hodnotách live tabuľku nezmení.', 'Bool'),
  uiApi('app_table_save', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k JSON poľu riadkov.'],
  ], 'Uloží celú tabuľku ako deterministické vnorené JSON pole.', 'Bool'),
  uiApi('app_table_save_schema', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k JSON poľu ôsmich typov stĺpcov.'],
  ], 'Uloží bounded schému `text/int/uint/bool` ako samostatný JSON súbor.', 'Bool'),
  uiApi('app_table_save_schema_atomic', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny schema súbor, ktorý sa atomicky nahradí.'],
  ], 'Zapíše schému do temporary_path a až potom atomicky nahradí target_path.', 'Bool'),
  uiApi('app_table_save_schema_full', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k full-schema JSON súboru.'],
  ], 'Uloží názvy a typy ôsmich stĺpcov pre typed-record model.', 'Bool'),
  uiApi('app_table_save_schema_full_atomic', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['temporary_path: String', 'Dočasný full-schema súbor.'],
    ['target_path: String', 'Finálny full-schema súbor.'],
  ], 'Uloží full schému a atomicky nahradí cieľový súbor.', 'Bool'),
  uiApi('app_table_save_atomic', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['temporary_path: String', 'Dočasný súbor v rovnakom filesystéme ako cieľ.'],
    ['target_path: String', 'Finálny JSON súbor, ktorý sa atomicky nahradí.'],
  ], 'Zapíše tabuľku do temporary_path a až potom atomicky nahradí target_path.', 'Bool'),
  uiApi('app_table_load', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k JSON poľu riadkov.'],
  ], 'Načíta tabuľku transakčne; neplatný JSON ponechá pôvodný model.', 'Bool'),
  uiApi('app_table_load_schema', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k JSON poľu ôsmich typov stĺpcov.'],
  ], 'Načíta schému transakčne a odmietne ju, ak nezodpovedá živým bunkám.', 'Bool'),
  uiApi('app_table_load_schema_full', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k full-schema JSON súboru.'],
  ], 'Načíta názvy a typy transakčne; pri chybe ponechá pôvodnú schému.', 'Bool'),
  uiApi('app_table_load_schema_full_if_version', [
    ['table_id: Int32', 'ID tabuľky v rozsahu `0..3`.'],
    ['path: String', 'Cesta k full-schema JSON súboru.'],
    ['expected_version: Int32', 'Verzia, ktorú musí súbor obsahovať.'],
  ], 'Načíta full-schema iba pri zhode verzie; pri nesúlade nič nezmení.', 'Bool'),
  uiApi('file_exists', [
    ['path: String', 'Cesta k súboru; natívny runtime ju prijíma ako UTF-8.'],
  ], 'Vráti `1`, ak cesta ukazuje na existujúci súbor alebo adresár.', 'Bool'),
  uiApi('file_flush', [
    ['path: String', 'Cesta k existujúcemu súboru.'],
  ], 'Vyžiada explicitný OS flush súboru. Je to hranica trvanlivosti, nie fsync adresára ani databázový commit.', 'Bool'),
  uiApi('file_lock', [
    ['path: String', 'Samostatná lock cesta, ktorú zdieľajú konkurenčné procesy.'],
  ], 'Skúsi bez čakania získať cross-process zámok a vráti neprázdny token pri úspechu.', 'UIntSize'),
  uiApi('file_unlock', [
    ['token: UIntSize', 'Token vrátený funkciou `file_lock`.'],
  ], 'Uvoľní natívny zámok; token po úspechu už nepoužívaj.', 'Bool'),
  uiApi('file_copy', [
    ['source_path: String', 'Existujúci zdrojový súbor.'],
    ['target_path: String', 'Nová cieľová cesta; existujúci cieľ operáciu odmietne.'],
  ], 'Skopíruje súbor bez tichej alokácie; pri chybe odstráni neúplný nový cieľ.', 'Bool'),
  uiApi('directory_exists', [
    ['path: String', 'Cesta k adresáru; natívny runtime ju prijíma ako UTF-8.'],
  ], 'Vráti `true`, iba ak cesta existuje a ukazuje na adresár.', 'Bool'),
  uiApi('directory_create', [
    ['path: String', 'Cesta nového adresára.'],
  ], 'Vytvorí adresár; existujúci adresár je úspech, súbor na ceste nie.', 'Bool'),
  uiApi('directory_delete', [
    ['path: String', 'Cesta k prázdnemu adresáru.'],
  ], 'Odstráni iba prázdny adresár a vráti `true` pri úspechu.', 'Bool'),
  uiApi('directory_list', [
    ['path: String', 'Directory path; entries are returned as UTF-8 names.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer for newline-separated entry names.'],
  ], 'Writes deterministic sorted file and subdirectory names without a trailing newline; writes nothing when capacity is insufficient.', 'UIntSize'),
  uiApi('directory_list_ex', [
    ['path: String', 'Directory path.'],
    ['names: write Slice<UInt8>', 'Caller-owned newline-separated UTF-8 entry names.'],
    ['kinds: write Slice<UInt8>', 'Parallel entry kinds: 1 file, 2 directory, 3 other.'],
  ], 'Returns the bounded entry count and writes names and kinds in the same deterministic byte-sorted order; writes neither output when capacity is insufficient.', 'UIntSize'),
  uiApi('file_size', [
    ['path: String', 'Cesta k existujúcemu súboru.'],
  ], 'Vráti veľkosť súboru v bajtoch alebo `0`, ak sa cesta nedá prečítať.', 'UIntSize'),
  uiApi('file_read', [
    ['path: String', 'Cesta k súboru.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer, do ktorého sa skopírujú bajty.'],
  ], 'Načíta najviac toľko bajtov, koľko má odovzdaný zapisovateľný buffer; vráti počet načítaných bajtov.', 'UIntSize'),
  uiApi('file_read_at', [
    ['path: String', 'File path.'],
    ['offset: UIntSize', 'Byte offset from the beginning; an offset beyond EOF returns 0.'],
    ['output: write Slice<UInt8>', 'Caller-owned chunk buffer.'],
  ], 'Reads at most the output capacity starting at offset and returns bytes read; does not write when the offset is past EOF.', 'UIntSize'),
  uiApi('file_read_text', [
    ['path: String', 'File path.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer; one extra byte is reserved for the trailing NUL.'],
  ], 'Reads the complete file only when it fits, writes a trailing NUL, and returns the UTF-8 byte length; returns 0 without writing when the file or capacity is invalid.', 'UIntSize'),
  uiApi('file_read_exact', [
    ['path: String', 'File path.'],
    ['output: write Slice<UInt8>', 'Caller-owned byte buffer; capacity must fit the complete file.'],
    ['output_length: write Slice<UIntSize>', 'Caller-owned one-element length output.'],
  ], 'Reads the complete file without a sentinel ambiguity and writes the byte length only on success; empty files are valid.', 'Bool'),
  uiApi('file_read_text_exact', [
    ['path: String', 'File path.'],
    ['output: write Slice<UInt8>', 'Caller-owned text buffer; one extra byte is reserved for the trailing NUL.'],
    ['output_length: write Slice<UIntSize>', 'Caller-owned one-element length output excluding the NUL.'],
  ], 'Reads the complete UTF-8 file, writes a trailing NUL and the byte length only on success; empty files are valid.', 'Bool'),
  uiApi('file_write_at', [
    ['path: String', 'File path; it is created when it does not exist.'],
    ['offset: UIntSize', 'Byte offset from the beginning; gaps are filled by the native filesystem.'],
    ['input: read Slice<UInt8>', 'Caller-owned bytes to write at the offset.'],
  ], 'Writes the complete input slice at offset without truncating the existing file and returns bytes written.', 'UIntSize'),
  uiApi('file_replace_atomic', [
    ['source_path: String', 'Hotový dočasný súbor, ktorý sa má presunúť.'],
    ['target_path: String', 'Cieľový súbor; existujúci obsah sa nahradí atomicky.'],
  ], 'Atomicky nahradí cieľový súbor pripraveným dočasným súborom. Použi po `file_write`.', 'Bool'),
  uiApi('file_write', [
    ['path: String', 'Cesta k súboru.'],
    ['input: read Slice<UInt8>', 'Caller-owned zdrojový pohľad na bajty, ktoré sa majú zapísať.'],
  ], 'Prepíše súbor odovzdanými bajtmi a vráti počet zapísaných bajtov.', 'UIntSize'),
  uiApi('file_write_text', [
    ['path: String', 'Cesta k súboru.'],
    ['content: String', 'UTF-8 text exportu, napríklad CSV alebo JSON fixture.'],
  ], 'Prepíše súbor UTF-8 textom a vráti počet zapísaných bajtov.', 'UIntSize'),
  uiApi('file_append_text', [
    ['path: String', 'Cesta k súboru.'],
    ['content: String', 'UTF-8 blok, ktorý sa pridá na koniec súboru.'],
  ], 'Pridá UTF-8 text na koniec súboru a vráti počet zapísaných bajtov.', 'UIntSize'),
  uiApi('file_append', [
    ['path: String', 'Cesta k súboru.'],
    ['input: read Slice<UInt8>', 'Caller-owned zdrojový buffer.'],
    ['length: UIntSize', 'Počet bajtov z bufferu, ktoré sa majú pridať.'],
  ], 'Pridá prefix caller-owned byte slice na koniec súboru.', 'UIntSize'),
  uiApi('format_int', [
    ['value: Int64', 'Signed integer na deterministické desiatkové formátovanie.'],
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer; odporúčaných je 20 bajtov.'],
  ], 'Zapíše signed decimal text bez alokácie a vráti počet bajtov; pri malom buffri vráti `0`.', 'UIntSize'),
  uiApi('format_uint', [
    ['value: UInt64', 'Unsigned integer na deterministické desiatkové formátovanie.'],
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer; odporúčaných je 20 bajtov.'],
  ], 'Zapíše unsigned decimal text bez alokácie a vráti počet bajtov.', 'UIntSize'),
  uiApi('format_bool', [
    ['value: Bool', 'Boolean hodnota.'],
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer s kapacitou aspoň 5 bajtov.'],
  ], 'Zapíše `true` alebo `false` bez alokácie a vráti počet bajtov.', 'UIntSize'),
  uiApi('format_float', [
    ['value: Float64', 'IEEE-754 hodnota na deterministické fixné formátovanie.'],
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer; pre bežné hodnoty odporúčaných je 32 bajtov.'],
  ], 'Zapíše šesť desatinných miest; `nan`, `inf` a `-inf` používa bez locale. Pri malom buffri alebo nepodporovanom rozsahu vráti `0`.', 'UIntSize'),
  uiApi('parse_int', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 byte buffer.'],
    ['input_length: UIntSize', 'Exact valid prefix length; it must not exceed the input capacity.'],
    ['output: write Slice<Int64>', 'Caller-owned output with capacity for at least one value.'],
  ], 'Parses a complete signed decimal value. Returns false for empty, malformed, or overflowing input and leaves output unchanged.', 'Bool'),
  uiApi('parse_uint', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 byte buffer.'],
    ['input_length: UIntSize', 'Exact valid prefix length; it must not exceed the input capacity.'],
    ['output: write Slice<UInt64>', 'Caller-owned output with capacity for at least one value.'],
  ], 'Parses a complete unsigned decimal value. Returns false for empty, malformed, negative, or overflowing input and leaves output unchanged.', 'Bool'),
  uiApi('parse_float', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 byte buffer.'],
    ['input_length: UIntSize', 'Exact valid prefix length; it must not exceed the input capacity.'],
    ['output: write Slice<Float64>', 'Caller-owned output with capacity for at least one value.'],
  ], 'Parses a finite JSON-style decimal value. Returns false for malformed or non-finite input and leaves output unchanged.', 'Bool'),
  uiApi('parse_bool', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 byte buffer.'],
    ['input_length: UIntSize', 'Exact valid prefix length; it must not exceed the input capacity.'],
    ['output: write Slice<Bool>', 'Caller-owned output with capacity for at least one value.'],
  ], 'Parses exactly lowercase `true` or `false`. Returns false otherwise and leaves output unchanged.', 'Bool'),
  uiApi('csv_escape', [
    ['value: String', 'Jeden UTF-8 CSV field bez predprípravy.'],
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer pre escapovaný field.'],
  ], 'Zapíše CSV field s automatickým quotingom a zdvojením úvodzoviek; pri malom buffri vráti `0`.', 'UIntSize'),
  uiApi('json_escape', [
    ['value: String', 'Jeden UTF-8 JSON string value bez predprípravy.'],
    ['output: write Slice<UInt8>', 'Caller-owned výstupný buffer pre JSON value vrátane úvodzoviek.'],
  ], 'Zapíše JSON string s escapovaním úvodzoviek, lomítok a control bytes; pri malom buffri vráti `0`.', 'UIntSize'),
  uiApi('json_object_field_string', [
    ['key: String', 'JSON object key.'],
    ['value: String', 'UTF-8 string value.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý member bez oddeľovacej čiarky.'],
  ], 'Zapíše celý JSON member `"key":"value"` s escapovaním oboch stringov. Čiarku medzi členmi pridáva caller.', 'UIntSize'),
  uiApi('json_object_field_int', [
    ['key: String', 'JSON object key.'],
    ['value: Int64', 'Signed integer zapisovaný ako JSON number.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý member bez čiarky.'],
  ], 'Zapíše escaped key a signed JSON integer bez alokácie. Nepridáva čiarku; pri malom buffri vráti `0`.', 'UIntSize'),
  uiApi('json_object_field_uint', [
    ['key: String', 'JSON object key.'],
    ['value: UInt64', 'Unsigned integer zapisovaný ako JSON number.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý member bez čiarky.'],
  ], 'Zapíše escaped key a unsigned JSON integer bez alokácie. Nepridáva čiarku; pri malom buffri vráti `0`.', 'UIntSize'),
  uiApi('json_object_field_float', [
    ['key: String', 'JSON object key.'],
    ['value: Float64', 'Konečné číslo zapisované fixným JSON formátom.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý member bez čiarky.'],
  ], 'Zapíše escaped key a konečný JSON float. `nan`/`inf` odmietne, aby výsledok ostal validný JSON.', 'UIntSize'),
  uiApi('json_object_field_bool', [
    ['key: String', 'JSON object key.'],
    ['value: Bool', 'Boolean value zapisovaná ako `true` alebo `false`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý member bez čiarky.'],
  ], 'Zapíše escaped key a JSON boolean bez alokácie; čiarku pridáva caller.', 'UIntSize'),
  uiApi('json_object_read_string', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre rozbalený UTF-8 string.'],
  ], 'Načíta jeden string member z flat JSON objectu bez alokácie; pri chybe alebo malom buffri vráti `0`.', 'UIntSize'),
  uiApi('json_object_read_int', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
  ], 'Načíta signed Int64 member z flat JSON objectu; neplatný alebo chýbajúci údaj vráti ako `0`.', 'Int64'),
  uiApi('json_object_read_uint', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
  ], 'Načíta unsigned UInt64 member z flat JSON objectu; neplatný alebo chýbajúci údaj vráti ako `0`.', 'UInt64'),
  uiApi('json_object_read_float', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
  ], 'Načíta konečný Float64 member z flat JSON objectu; neplatný, chýbajúci alebo non-finite údaj vráti ako `0.0`.', 'Float64'),
  uiApi('json_object_read_bool', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
  ], 'Načíta boolean member z flat JSON objectu; `true` je `true`, ostatné alebo neplatné hodnoty sú `false`.', 'Bool'),
  uiApi('json_object_read_string_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre rozbalený string.'],
    ['output_length: write Slice<UIntSize>', 'Jednoprvkový caller-owned výstup skutočnej dĺžky.'],
  ], 'Načíta string bez sentinelovej nejednoznačnosti. Vracia `Bool`, povoľuje aj prázdny string a pri chybe nemení výstupy.', 'Bool'),
  uiApi('json_object_read_int_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
    ['output: write Slice<Int64>', 'Jednoprvkový caller-owned výstup signed hodnoty.'],
  ], 'Načíta Int64 a rozlíši platnú nulu od chýbajúceho alebo neplatného membera.', 'Bool'),
  uiApi('json_object_read_uint_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
    ['output: write Slice<UInt64>', 'Jednoprvkový caller-owned výstup unsigned hodnoty.'],
  ], 'Načíta UInt64 a rozlíši platnú nulu od chýbajúceho alebo neplatného membera.', 'Bool'),
  uiApi('json_object_read_float_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
    ['output: write Slice<Float64>', 'Jednoprvkový caller-owned výstup konečného Float64.'],
  ], 'Načíta konečný Float64 a pri chybe zachová pôvodný output.', 'Bool'),
  uiApi('json_object_read_bool_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 JSON object bytes.'],
    ['key: String', 'Flat object key bez okolitého JSON quoting.'],
    ['output: write Slice<Bool>', 'Jednoprvkový caller-owned výstup boolean hodnoty.'],
  ], 'Načíta explicitné `true` alebo `false`; platné `false` sa už nezamení za chybu.', 'Bool'),
  uiApi('json_array_int', [
    ['values: read Slice<Int64>', 'Caller-owned signed integer array alebo slice.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý JSON array.'],
  ], 'Zapíše bounded JSON array signed integerov vrátane `[` a `]`; pri malom buffri vráti `0`.', 'UIntSize'),
  uiApi('json_array_uint', [
    ['values: read Slice<UInt64>', 'Caller-owned unsigned integer array alebo slice.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý JSON array.'],
  ], 'Zapíše bounded JSON array unsigned integerov bez alokácie.', 'UIntSize'),
  uiApi('json_array_float', [
    ['values: read Slice<Float64>', 'Caller-owned konečný float array alebo slice.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý JSON array.'],
  ], 'Zapíše bounded JSON array fixných floatov; `nan`/`inf` odmietne kvôli validnému JSON.', 'UIntSize'),
  uiApi('json_array_bool', [
    ['values: read Slice<Bool>', 'Caller-owned boolean array alebo slice.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý JSON array.'],
  ], 'Zapíše bounded JSON array `true`/`false` hodnôt bez alokácie.', 'UIntSize'),
  uiApi('http_response_write', [
    ['status: UInt16', 'HTTP status 100 až 999; známe kódy dostanú štandardný reason phrase.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes, napríklad `text/plain`.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP/1.1 odpoveď.'],
  ], 'Zostaví bounded HTTP/1.1 response s Content-Length a Connection: close. Nevykonáva sieťové I/O ani TLS.', 'UIntSize'),
  uiApi('http_response_write_ex', [
    ['status: UInt16', 'HTTP status 100 až 999; známe kódy dostanú štandardný reason phrase.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes, napríklad `text/plain`.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['keep_alive: Bool', 'Explicitne zvolí `Connection: keep-alive` alebo `Connection: close`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP/1.1 odpoveď.'],
  ], 'Rozšírený response writer s explicitným connection lifecycle; pri malom buffri nevytvorí partial output a nevykonáva socket I/O.', 'UIntSize'),
  uiApi('http_response_write_header', [
    ['status: UInt16', 'HTTP status 100 až 999.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes.'],
    ['header_name: String', 'Jedna tokenová custom hlavička, napríklad `Retry-After`.'],
    ['header_value: String', 'ASCII hodnota bez CR/LF alebo iných control bytes.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP odpoveď.'],
  ], 'Zostaví response s jednou validovanou custom hlavičkou; pri malom buffri nevykoná partial write.', 'UIntSize'),
  uiApi('http_response_write_header_ex', [
    ['status: UInt16', 'HTTP status 100 až 999.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes.'],
    ['header_name: String', 'Jedna tokenová custom hlavička.'],
    ['header_value: String', 'ASCII hodnota bez CR/LF alebo iných control bytes.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['keep_alive: Bool', 'Explicitne zvolí `Connection: keep-alive` alebo `Connection: close`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP odpoveď.'],
  ], 'Response writer s jednou custom hlavičkou a explicitným connection lifecycle; nevykonáva socket I/O.', 'UIntSize'),
  uiApi('http_response_write_header_block', [
    ['status: UInt16', 'HTTP status 100 až 999.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes.'],
    ['headers: String', 'CRLF-separated blok `Name: value\\r\\nOther: value`; framingové hlavičky sú zakázané.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP odpoveď.'],
  ], 'Zostaví response s viacerými validovanými custom hlavičkami; pri malformed riadku alebo malom buffri nevykoná partial write.', 'UIntSize'),
  uiApi('http_response_write_header_block_ex', [
    ['status: UInt16', 'HTTP status 100 až 999.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes.'],
    ['headers: String', 'CRLF-separated blok validovaných custom hlavičiek.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['keep_alive: Bool', 'Explicitne zvolí `Connection: keep-alive` alebo `Connection: close`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP odpoveď.'],
  ], 'Response writer s blokom custom hlavičiek a explicitným connection lifecycle; nevykonáva socket I/O.', 'UIntSize'),
  uiApi('http_response_write_cookie', [
    ['status: UInt16', 'HTTP status 100 až 999.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes.'],
    ['cookie_name: String', 'Cookie token name.'],
    ['cookie_value: String', 'Cookie-octet value; control bytes, separators and backslash are rejected.'],
    ['cookie_attributes: String', 'Optional ASCII attribute suffix such as `Path=/; HttpOnly`.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP odpoveď.'],
  ], 'Zostaví response s dynamickou validovanou `Set-Cookie` hlavičkou; nealokuje ani nevykonáva socket I/O.', 'UIntSize'),
  uiApi('http_response_write_cookie_ex', [
    ['status: UInt16', 'HTTP status 100 až 999.'],
    ['content_type: String', 'ASCII Content-Type bez control bytes.'],
    ['cookie_name: String', 'Cookie token name.'],
    ['cookie_value: String', 'Cookie-octet value; control bytes, separators and backslash are rejected.'],
    ['cookie_attributes: String', 'Optional ASCII attribute suffix such as `Path=/; HttpOnly`.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo odpovede.'],
    ['keep_alive: Bool', 'Explicitne zvolí `Connection: keep-alive` alebo `Connection: close`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celú HTTP odpoveď.'],
  ], 'Dynamická `Set-Cookie` response hlavička s explicitným connection lifecycle; nevykonáva socket I/O.', 'UIntSize'),
  uiApi('http_response_status', [
    ['input: read Slice<UInt8>', 'Caller-owned HTTP response bytes.'],
  ], 'Validates an HTTP/1.1 status line and returns its three-digit status code, or 0 for invalid input.', 'UInt16'),
  uiApi('http_response_status_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned receive buffer.'],
    ['input_length: UIntSize', 'Number of valid response bytes in the buffer.'],
  ], 'Reads only the explicit response prefix and returns the status code, or 0 for invalid or incomplete input.', 'UInt16'),
  uiApi('http_response_header', [
    ['input: read Slice<UInt8>', 'Caller-owned HTTP response bytes.'],
    ['name: String', 'Case-insensitive response header name.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the header value.'],
  ], 'Copies one response header without allocation or socket I/O; 0 means missing, invalid, or too small.', 'UIntSize'),
  uiApi('http_response_header_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned receive buffer.'],
    ['input_length: UIntSize', 'Number of valid response bytes in the buffer.'],
    ['name: String', 'Case-insensitive response header name.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the header value.'],
  ], 'Copies one header from an explicit response prefix and rejects incomplete framing or a small output.', 'UIntSize'),
  uiApi('http_response_header_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned HTTP response bytes.'],
    ['name: String', 'Case-insensitive response header name.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the header value.'],
    ['output_length: write Slice<UIntSize>', 'One-element caller-owned length output written only on success.'],
  ], 'Returns Bool, supports an empty header value, and performs no partial write when the header is missing or the output is too small.', 'Bool'),
  uiApi('http_response_body', [
    ['input: read Slice<UInt8>', 'Caller-owned HTTP response bytes.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the body.'],
  ], 'Validates Content-Length and copies the complete response body; chunked transfer is rejected.', 'UIntSize'),
  uiApi('http_response_body_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned receive buffer.'],
    ['input_length: UIntSize', 'Number of valid response bytes in the buffer.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the body.'],
  ], 'Reads a bounded body from an explicit response prefix without partial output or allocation.', 'UIntSize'),
  uiApi('http_response_body_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned complete HTTP response bytes.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the body.'],
    ['output_length: write Slice<UIntSize>', 'One-element caller-owned Content-Length output written only on success.'],
  ], 'Returns Bool, preserves a valid empty body, rejects chunked or incomplete framing, and performs no partial write.', 'Bool'),
  uiApi('http_response_body_chunked_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned complete chunked HTTP response bytes.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the decoded body.'],
    ['output_length: write Slice<UIntSize>', 'One-element caller-owned decoded length written only on success.'],
  ], 'Decodes a complete Transfer-Encoding: chunked response, validates trailers, and performs no partial write.', 'Bool'),
  uiApi('http_request_body_chunked_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned complete chunked HTTP request bytes.'],
    ['output: write Slice<UInt8>', 'Caller-owned destination for the decoded body.'],
    ['output_length: write Slice<UIntSize>', 'One-element caller-owned decoded length written only on success.'],
  ], 'Decodes a complete Transfer-Encoding: chunked request, validates trailers, and performs no partial write.', 'Bool'),
  uiApi('http_request_write', [
    ['method: String', 'HTTP token, napríklad `GET` alebo `POST`.'],
    ['target: String', 'ASCII request target, napríklad `/api/items`.'],
    ['host: String', 'ASCII Host hlavička bez control bytes.'],
    ['body: read Slice<UInt8>', 'Caller-owned telo požiadavky.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý HTTP/1.1 request.'],
  ], 'Zostaví bounded request s Host, Content-Length a Connection: close. Nevykonáva sieťové I/O ani TLS.', 'UIntSize'),
  uiApi('http_request_write_prefix', [
    ['method: String', 'HTTP token, napríklad `GET` alebo `POST`.'],
    ['target: String', 'ASCII request target, napríklad `/api/items`.'],
    ['host: String', 'ASCII Host hlavička bez control bytes.'],
    ['body: read Slice<UInt8>', 'Caller-owned pracovný buffer tela.'],
    ['body_length: UIntSize', 'Platná dĺžka prefixu; nesmie byť väčšia než kapacita body.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre celý HTTP/1.1 request.'],
  ], 'Zostaví bounded request z explicitného platného prefixu tela. Zapíše presný Content-Length a nevykonáva sieťové I/O ani TLS.', 'UIntSize'),
  uiApi('http_request_write_header_block', [
    ['method: String', 'HTTP method token.'],
    ['target: String', 'Request target.'],
    ['host: String', 'Host header value.'],
    ['headers: String', 'CRLF-separated custom header block; framing names are rejected.'],
    ['body: read Slice<UInt8>', 'Caller-owned request body bytes.'],
    ['output: write Slice<UInt8>', 'Caller-owned serialized request buffer.'],
  ], 'Serializes a request with a bounded custom-header block.', 'UIntSize'),
  uiApi('bearer_token_matches', [
    ['request: read Slice<UInt8>', 'Caller-owned complete HTTP request bytes.'],
    ['expected_token: read Slice<UInt8>', 'Caller-owned bearer token bytes.'],
    ['token_length: UIntSize', 'Exact token prefix length.'],
    ['scratch: write Slice<UInt8>', 'Caller-owned header scratch buffer.'],
  ], 'Compares an exact Bearer token without storing authentication state.', 'Bool'),
  uiApi('cookie_value_matches', [
    ['request: read Slice<UInt8>', 'Caller-owned complete HTTP request bytes.'],
    ['cookie_name: read Slice<UInt8>', 'Caller-owned cookie name bytes.'],
    ['cookie_name_length: UIntSize', 'Exact cookie name length.'],
    ['expected_value: read Slice<UInt8>', 'Caller-owned expected cookie value bytes.'],
    ['expected_value_length: UIntSize', 'Exact expected value length.'],
    ['scratch: write Slice<UInt8>', 'Caller-owned Cookie header scratch buffer.'],
  ], 'Matches one exact Cookie name=value pair without allocation or session state.', 'Bool'),
  uiApi('http_request_append', [
    ['output: write Slice<UInt8>', 'Caller-owned akumulačný buffer requestu.'],
    ['current_length: UIntSize', 'Počet už uložených bajtov v output buffri.'],
    ['chunk: read Slice<UInt8>', 'Caller-owned prijatý fragment.'],
    ['chunk_length: UIntSize', 'Presný počet platných bajtov z fragmentu.'],
  ], 'Pripojí fragment bez alokácie a vráti novú dĺžku; pri nedostatku kapacity alebo neplatnom prefixe ponechá output nezmenený.', 'UIntSize'),
  uiApi('http_request_frame_length_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned request buffer, ktorý môže obsahovať viac requestov.'],
    ['input_length: UIntSize', 'Presný počet platných bajtov v buffri.'],
  ], 'Vráti presnú dĺžku prvého HTTP/1.1 request frame vrátane Content-Length tela; `0` znamená neplatný alebo neúplný frame. Transfer-Encoding sa odmieta.', 'UIntSize'),
  uiApi('http_request_chunked_frame_length_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned buffer s akumulovanými request fragmentmi.'],
    ['input_length: UIntSize', 'Presný počet platných bajtov v buffri.'],
  ], 'Vráti presnú hranicu prvého kompletného `Transfer-Encoding: chunked` requestu; pri neúplnom alebo neplatnom framingu vráti 0 a suffix za frame ponechá na ďalšie spracovanie.', 'UIntSize'),
  uiApi('http_request_consume_prefix', [
    ['input: write Slice<UInt8>', 'Caller-owned buffer s requestmi za sebou.'],
    ['input_length: UIntSize', 'Aktuálny počet platných bajtov v buffri.'],
    ['consumed_length: UIntSize', 'Počet bajtov prvého spracovaného frame na odstránenie.'],
  ], 'Odstráni spracovaný prefix posunom zvyšných bajtov na začiatok buffra a vráti novú dĺžku.', 'UIntSize'),
  uiApi('http_request_is_complete_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned request buffer, ktorý môže byť väčší než aktuálne dáta.'],
    ['input_length: UIntSize', 'Presný počet platných request bajtov v buffri.'],
  ], 'Validuje iba prefix buffra do input_length; odmietne dĺžku väčšiu než slice.', 'Bool'),
  uiApi('http_route_match_prefix', [
    ['input: read Slice<UInt8>', 'Caller-owned request buffer.'],
    ['input_length: UIntSize', 'Presný počet platných request bajtov.'],
    ['expected_method: String', 'Presná HTTP metóda, napríklad GET.'],
    ['expected_target: String', 'Presný request target, napríklad /hello.'],
  ], 'Porovná route iba v platnom prefixe buffra; nealokuje a nevykonáva I/O.', 'Bool'),
  uiApi('http_request_is_complete', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP bajty.'],
  ], 'Overí validnú HTTP/1.1 request line, hlavičky a celé Content-Length telo; odmietne Transfer-Encoding.', 'Bool'),
  uiApi('http_request_keep_alive', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP request bajty.'],
  ], 'Vráti `true` pre validný request bez `Connection: close` alebo s `keep-alive`; neznámy či duplicitný režim odmietne.', 'Bool'),
  uiApi('http_request_method', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP bajty.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre metódu, napríklad `GET`.'],
  ], 'Skopíruje HTTP metódu do bounded buffra; pri neplatnom alebo malom buffri vráti `0`.', 'UIntSize'),
  uiApi('http_request_target', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP bajty.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre request target, napríklad `/api/items`.'],
  ], 'Skopíruje request target z HTTP/1.1 request line bez alokácie.', 'UIntSize'),
  uiApi('http_request_header', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP bajty.'],
    ['name: String', 'Case-insensitive názov hlavičky, napríklad `Host`.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre hodnotu hlavičky.'],
  ], 'Nájde jednu HTTP hlavičku a skopíruje jej trimovanú hodnotu; duplicitná hlavička sa odmietne.', 'UIntSize'),
  uiApi('http_request_body', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP bajty.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre Content-Length telo.'],
  ], 'Skopíruje bounded telo podľa jedinej Content-Length; chunked Transfer-Encoding nie je podporovaný.', 'UIntSize'),
  uiApi('http_query_param', [
    ['input: read Slice<UInt8>', 'Caller-owned query alebo HTTP request target.'],
    ['key: String', 'Presný ASCII názov query parametra.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre percent-dekodovanú hodnotu.'],
  ], 'Nájde jeden query parameter, dekóduje `%HH` a `+`; duplicitný alebo neplatný parameter odmietne.', 'UIntSize'),
  uiApi('http_query_param_exact', [
    ['input: read Slice<UInt8>', 'Caller-owned query alebo HTTP request target.'],
    ['key: String', 'Presný ASCII názov query parametra.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre percent-dekodovanú hodnotu.'],
    ['output_length: write Slice<UIntSize>', 'Caller-owned jedno-prvkové pole pre presnú dĺžku.'],
  ], 'Vráti `Bool`, rozlíši aj platnú prázdnu hodnotu a pri chybe nemení výstupy.', 'Bool'),
  uiApi('http_route_match', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP bajty.'],
    ['expected_method: String', 'Presná HTTP metóda, napríklad `GET`.'],
    ['expected_target: String', 'Presný request target, napríklad `/hello`.'],
  ], 'Bez alokácie porovná request line s metódou a targetom; nevykonáva sieťové I/O.', 'Bool'),
  uiApi('http_router_clear', [], 'Vymaže 16-slotovú caller-driven HTTP route tabuľku bez socket I/O.', 'Unit'),
  uiApi('http_router_add', [
    ['method: String', 'Presná HTTP metóda, napríklad `GET`.'],
    ['target: String', 'Presný request target, napríklad `/hello`.'],
    ['status: UInt16', 'HTTP status kód odpovede.'],
    ['content_type: String', 'ASCII Content-Type odpovede.'],
    ['body: read Slice<UInt8>', 'Caller-owned bounded telo odpovede.'],
  ], 'Vloží alebo nahradí exact route; pri plnej tabuľke alebo neplatných limitoch vráti `false`.', 'Bool'),
  uiApi('http_router_add_exact', [
    ['method: String', 'Presná HTTP metóda, napríklad `GET`.'],
    ['target: String', 'Presný HTTP request target.'],
    ['status: UInt16', 'HTTP status kód odpovede.'],
    ['content_type: String', 'ASCII Content-Type odpovede.'],
    ['body: read Slice<UInt8>', 'Caller-owned buffer s response payloadom.'],
    ['body_length: UIntSize', 'Počet platných bajtov; nesmie prekročiť kapacitu slice.'],
  ], 'Vloží exact route iba s explicitným prefixom body; nepoužitý koniec caller-owned slice sa neodošle.', 'Bool'),
  uiApi('http_router_add_prefix', [
    ['method: String', 'Presná HTTP metóda route, napríklad `GET`.'],
    ['target_prefix: String', 'Prefix request targetu, napríklad `/api/time-entry/`.'],
    ['status: UInt16', 'HTTP status kód odpovede.'],
    ['content_type: String', 'ASCII Content-Type odpovede.'],
    ['body: read Slice<UInt8>', 'Caller-owned bounded telo odpovede.'],
  ], 'Vloží alebo nahradí prefix route; exact route má vždy prednosť a prefixy vyhrá najdlhší.', 'Bool'),
  uiApi('http_router_remove', [
    ['method: String', 'Presná HTTP metóda route, napríklad `GET`.'],
    ['target: String', 'Presný request target route, napríklad `/hello`.'],
  ], 'Odstráni presnú route; vráti `false`, ak route neexistuje alebo sú vstupy neplatné.', 'Bool'),
  uiApi('http_router_remove_prefix', [
    ['method: String', 'Presná HTTP metóda prefix route, napríklad `GET`.'],
    ['target_prefix: String', 'Rovnaký prefix request targetu, ktorý bol registrovaný.'],
  ], 'Odstráni jednu prefix route; vráti `false`, ak neexistuje.', 'Bool'),
  uiApi('http_router_respond', [
    ['input: read Slice<UInt8>', 'Caller-owned prijaté HTTP request bajty.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre serializovanú odpoveď.'],
  ], 'Vyberie route podľa request line a zapíše odpoveď; pri miss vytvorí bounded 404.', 'UIntSize'),
  uiApi('http_router_respond_prefix', [
    ['request: read Slice<UInt8>', 'Caller-owned receive buffer obsahujúci HTTP request.'],
    ['request_length: UIntSize', 'Počet platných request bajtov; nesmie prekročiť kapacitu slice.'],
    ['output: write Slice<UInt8>', 'Caller-owned bounded buffer pre serializovanú odpoveď.'],
  ], 'Spracuje iba explicitný platný prefix requestu a zapíše route odpoveď alebo bounded 404.', 'UIntSize'),
  uiApi('http_router_count', [], 'Vráti počet aktívnych exact route entries.', 'UIntSize'),
  uiApi('http_session_open', [
    ['listener: UIntSize', 'TCP listener token z `net_tcp_listen`.'],
    ['max_connections: UInt32', 'Maximálne `1..8` aktívnych connection slotov.'],
    ['max_header_bytes: UInt32', 'Limit request line + hlavičky; najviac 8192 bajtov.'],
    ['max_body_bytes: UInt32', 'Limit Content-Length tela; najviac 8192 bajtov.'],
  ], 'Otvorí bounded HTTP session bez alokácie. Session vlastní listener a pri close zatvorí všetky sloty.', 'UIntSize'),
  uiApi('http_session_open_tls', [
    ['listener: UIntSize', 'TCP listener token z `net_tcp_listen`.'],
    ['max_connections: UInt32', 'Maximálne `1..8` aktívnych connection slotov.'],
    ['max_header_bytes: UInt32', 'Limit request line + hlavičky; najviac 8192 bajtov.'],
    ['max_body_bytes: UInt32', 'Limit Content-Length tela; najviac 8192 bajtov.'],
    ['certificate: String', 'Windows: PFX/PKCS#12 cesta; Linux: PEM certifikát.'],
    ['private_key: String', 'Windows: heslo PFX; Linux: PEM privátny kľúč.'],
  ], 'Otvorí bounded HTTP session, ktorá pri prijatí spojenia automaticky vykoná TLS handshake a šifrovaný route/read/write.', 'UIntSize'),
  uiApi('http_session_step', [
    ['session: UIntSize', 'Token vrátený z `http_session_open`.'],
    ['timeout_ms: UInt32', 'Per-step receive/accept timeout; `0` je normalizovaný na krátky polling.'],
  ], 'Vykoná najviac jeden bounded accept/read/route/write krok. Vráti `1` pre keep-alive response, `2` pre ukončený slot a `0` pri idle alebo neúplnej požiadavke.', 'UInt32'),
  uiApi('http_session_close', [
    ['session: UIntSize', 'Session token.'],
  ], 'Zatvorí session-owned connection sloty aj listener a vykoná cleanup.', 'Bool'),
  uiApi('net_tls_open_client', [
    ['socket: UIntSize', 'Existujúci TCP socket token. TLS ho po úspešnom open vlastní.'],
    ['server_name: String', 'Server name pre TLS handshake a hostname validation.'],
    ['verify_peer: Bool', 'Peer certifikát validuj cez platformový trust store; `false` používaj iba v lokálnom teste.'],
  ], 'Vytvorí bounded platformový TLS client context. Windows používa Schannel, Linux natívny OpenSSL backend.', 'UIntSize'),
  uiApi('net_tls_open_server', [
    ['socket: UIntSize', 'Existujúci TCP socket token z `net_tcp_accept`; TLS server ho po open vlastní.'],
    ['certificate: String', 'Windows: cesta k PFX/PKCS#12 certifikátu; Linux: cesta k PEM certifikátu.'],
    ['private_key: String', 'Windows: heslo PFX; Linux: cesta k PEM privátnemu kľúču.'],
  ], 'Vytvorí bounded TLS server context. PFX/PEM credential provisioning prebieha iba z explicitných ciest; token vlastní socket.', 'UIntSize'),
  uiApi('net_tls_step', [
    ['tls: UIntSize', 'Token z `net_tls_open_client`.'],
    ['timeout_ms: UInt32', 'Timeout pre handshake I/O; `0` znamená krátky polling.'],
  ], 'Posunie handshake a vráti `1` progress, `2` open, `3` peer close alebo `4` error.', 'UInt32'),
  uiApi('net_tls_state', [
    ['tls: UIntSize', 'TLS token.'],
  ], 'Vráti stav `0=closed`, `1=handshaking`, `2=open`, `3=error`, `4=peer closed`.', 'UInt32'),
  uiApi('net_tls_error', [
    ['tls: UIntSize', 'TLS token.'],
  ], 'Vráti posledný natívny TLS status/error kód.', 'Int32'),
  uiApi('net_tls_send', [
    ['tls: UIntSize', 'Otvorený TLS token.'],
    ['input: read Slice<UInt8>', 'Caller-owned payload pre šifrovaný bounded zápis.'],
  ], 'Zašifruje a odošle jeden payload; pri chybe vráti `0`.', 'UIntSize'),
  uiApi('net_tls_receive', [
    ['tls: UIntSize', 'Otvorený TLS token.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre dešifrované dáta.'],
  ], 'Prijme a dešifruje jeden bounded aplikačný blok; pri peer-close alebo chybe vráti `0`.', 'UIntSize'),
  uiApi('net_tls_close', [
    ['tls: UIntSize', 'TLS token.'],
  ], 'Ukončí TLS context a zatvorí socket, ktorý adapter vlastní.', 'Bool'),
  uiApi('net_tcp_connect', [
    ['host: String', 'IPv4 adresa alebo `localhost`; DNS mená zatiaľ nie sú podporované.'],
    ['port: UInt16', 'TCP port cieľového loopback alebo IPv4 servera.'],
  ], 'Otvorí blokujúce TCP spojenie a vráti nepriehľadný socket token; pri zlyhaní vráti `0`.', 'UIntSize'),
  uiApi('net_tcp_connect_dns', [
    ['host: String', 'Hostname alebo služba, ktorú natívny OS resolver preloží na IPv4.'],
    ['port: UInt16', 'TCP port cieľa.'],
  ], 'Blokujúco vyrieši hostname cez natívny resolver a otvorí IPv4 TCP spojenie. DNS timeout/availability je zodpovednosť platformy.', 'UIntSize'),
  uiApi('net_tcp_listen', [
    ['port: UInt16', 'Lokálny TCP port; server počúva iba na `127.0.0.1`.'],
  ], 'Vytvorí blokujúci TCP listener na loopback adrese s backlogom 8.', 'UIntSize'),
  uiApi('net_tcp_accept', [
    ['listener: UIntSize', 'Token vrátený z `net_tcp_listen`.'],
  ], 'Blokujúco prijme jedno TCP spojenie a vráti nový socket token.', 'UIntSize'),
  uiApi('net_tcp_send', [
    ['socket: UIntSize', 'Token spojenia z `net_tcp_connect` alebo `net_tcp_accept`.'],
    ['input: read Slice<UInt8>', 'Caller-owned zdrojový buffer; odošle sa najviac jeden bounded chunk.'],
  ], 'Blokujúco odošle jeden chunk bajtov a vráti počet odoslaných bajtov.', 'UIntSize'),
  uiApi('net_tcp_send_prefix', [
    ['socket: UIntSize', 'Token spojenia z `net_tcp_connect` alebo `net_tcp_accept`.'],
    ['input: read Slice<UInt8>', 'Caller-owned zdrojový buffer; musí obsahovať aspoň `length` bajtov.'],
    ['length: UIntSize', 'Presný počet bajtov z prefixu buffra, ktorý sa má odoslať.'],
  ], 'Blokujúco odošle presne určený prefix buffra; vráti počet skutočne odoslaných bajtov.', 'UIntSize'),
  uiApi('net_tcp_receive', [
    ['socket: UIntSize', 'Token spojenia z `net_tcp_connect` alebo `net_tcp_accept`.'],
    ['output: write Slice<UInt8>', 'Caller-owned zapisovateľný buffer pre prijaté bajty.'],
  ], 'Blokujúco prijme jeden chunk do caller-owned buffra a vráti počet bajtov.', 'UIntSize'),
  uiApi('net_socket_set_timeout', [
    ['socket: UIntSize', 'Token socketu z `net_tcp_connect`, `net_tcp_listen` alebo `net_tcp_accept`.'],
    ['milliseconds: UInt32', 'Timeout pre čítanie aj zápis; `0` timeout vypne.'],
  ], 'Nastaví explicitný receive/send timeout socketu. Pri chybe vráti `false`; timeout neodstraňuje blokujúci charakter API.', 'Bool'),
  uiApi('net_socket_close', [
    ['socket: UIntSize', 'Token socketu; po zatvorení ho už nepoužívaj.'],
  ], 'Zatvorí TCP socket a vráti `true` pri úspechu. TLS, DNS a async API sú samostatné etapy.', 'Bool'),
  uiApi('net_reactor_open', [
    ['max_watches: UInt32', 'Maximum aktívnych socket watchov; najviac 64.'],
    ['max_events: UInt32', 'Maximum udalostí v jednom poll; najviac 64.'],
  ], 'Vytvorí bounded natívny readiness reactor. Reactor nevlastní sockety.', 'UIntSize'),
  uiApi('net_reactor_watch', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token z TCP API.'],
    ['interest: UInt32', 'Bity: `1=readable`, `2=writable`, `4=error`, `8=peer closed`.'],
    ['user_data: UIntSize', 'Caller-owned identifikátor vrátený s udalosťou.'],
  ], 'Pridá alebo aktualizuje watch. Pri opakovanom sockete sa nastavenie nahradí.', 'Bool'),
  uiApi('net_reactor_unwatch', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token registrovaný vo watch.'],
  ], 'Zruší watch bez zatvorenia socketu; je to cancel-safe cleanup operácia.', 'Bool'),
  uiApi('net_reactor_poll', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['timeout_ms: UInt32', 'Maximálne čakanie v milisekundách.'],
  ], 'Počká na readiness a vráti počet udalostí; `0` znamená timeout alebo žiadnu udalosť.', 'UInt32'),
  uiApi('net_reactor_event_socket', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['index: UInt32', 'Index udalosti z posledného pollu.'],
  ], 'Vráti socket token udalosti.', 'UIntSize'),
  uiApi('net_reactor_event_flags', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['index: UInt32', 'Index udalosti z posledného pollu.'],
  ], 'Vráti readiness bity udalosti.', 'UInt32'),
  uiApi('net_reactor_event_bytes', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['index: UInt32', 'Index udalosti z posledného pollu.'],
  ], 'Vráti počet prenesených bajtov buffered operácie; pri watch udalosti je `0`.', 'UIntSize'),
  uiApi('net_reactor_event_user', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['index: UInt32', 'Index udalosti z posledného pollu.'],
  ], 'Vráti caller `user_data` priradené k udalosti.', 'UIntSize'),
  uiApi('net_reactor_error', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
  ], 'Vráti posledný natívny error kód reactora; pre neplatný token vráti `-1`.', 'Int32'),
  uiApi('net_reactor_close', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
  ], 'Uvoľní reactor state, ale nezatvára watched sockety.', 'Bool'),
  uiApi('net_reactor_submit_accept', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['listener: UIntSize', 'TCP listener token.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Naplánuje bounded asynchrónny accept; výsledná udalosť vráti nový socket token.', 'UIntSize'),
  uiApi('net_reactor_submit_connect', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['host: String', 'IPv4 adresa alebo `localhost`.'],
    ['port: UInt16', 'TCP port cieľa.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Naplánuje asynchrónny IPv4 connect bez uchovania caller pamäte.', 'UIntSize'),
  uiApi('net_reactor_submit_receive', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Armne jednorazovú receive-readiness operáciu; po udalosti zavolaj `net_tcp_receive`.', 'UIntSize'),
  uiApi('net_reactor_submit_send', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Armne jednorazovú send-readiness operáciu; po udalosti zavolaj `net_tcp_send`.', 'UIntSize'),
  uiApi('net_reactor_submit_receive_buffer', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer, do ktorého sa zapíšu prijaté dáta.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Naplánuje natívny buffered receive; počet bajtov prečítaj cez `net_reactor_event_bytes`.', 'UIntSize'),
  uiApi('net_reactor_submit_send_buffer', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token.'],
    ['input: read Slice<UInt8>', 'Caller-owned dáta na odoslanie.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Naplánuje natívny buffered send; počet odoslaných bajtov prečítaj cez `net_reactor_event_bytes`.', 'UIntSize'),
  uiApi('net_reactor_submit_send_buffer_prefix', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['socket: UIntSize', 'Socket token.'],
    ['input: read Slice<UInt8>', 'Caller-owned dáta na odoslanie.'],
    ['length: UIntSize', 'Presný počet bajtov z prefixu slice, väčší než nula a najviac dĺžka slice.'],
    ['user_data: UIntSize', 'Caller identifikátor udalosti.'],
  ], 'Naplánuje buffered send presného prefixu; počet odoslaných bajtov prečítaj cez `net_reactor_event_bytes`.', 'UIntSize'),
  uiApi('net_reactor_cancel', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['operation: UIntSize', 'Token z `net_reactor_submit_*`.'],
  ], 'Požiada platformu o zrušenie operácie; dokončenie môže byť ešte doručené ako error event.', 'Bool'),
  uiApi('net_reactor_event_operation', [
    ['reactor: UIntSize', 'Token z `net_reactor_open`.'],
    ['index: UInt32', 'Index udalosti z posledného pollu.'],
  ], 'Vráti one-shot operation token udalosti alebo `0` pri readiness watch udalosti.', 'UIntSize'),
  uiApi('file_delete', [
    ['path: String', 'Cesta k súboru.'],
  ], 'Odstráni súbor a vráti `1` pri úspechu.', 'Bool'),
  uiApi('ui_select', [
    ['event_id: Int32', 'ID odoslané pri zmene výberu.'], ['x: Int32', 'Vodorovná pozícia.'],
    ['y: Int32', 'Zvislá pozícia.'], ['width: Int32', 'Šírka výberu.'],
    ['height: Int32', 'Výška výberu.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'],
  ], 'Vytvorí select. Položky pridaj cez `ui_select_option`.'),
  uiApi('ui_select_option', [
    ['event_id: Int32', 'ID selectu.'], ['label: String', 'Text novej voľby.'],
  ], 'Pridá voľbu do existujúceho selectu.'),
  uiApi('ui_select_index', [
    ['event_id: Int32', 'ID selectu.'],
  ], 'Vráti index aktuálne vybranej voľby od nuly.', 'Int32'),
  uiApi('ui_select_set_index', [
    ['event_id: Int32', 'ID selectu.'], ['selected_index: Int32', 'Index voľby od nuly.'],
  ], 'Programovo vyberie voľbu selectu.'),
  uiApi('ui_scroll_panel', [
    ['text: String', 'Read-only text panelu.'], ['x: Int32', 'Vodorovná pozícia.'],
    ['y: Int32', 'Zvislá pozícia.'], ['width: Int32', 'Šírka panelu.'],
    ['height: Int32', 'Výška panelu.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'], ['corner_radius: Int32', 'Zaoblenie rohov.'],
  ], 'Vytvorí scrollovateľný read-only textový panel.'),
  uiApi('ui_list', [
    ['event_id: Int32', 'ID odoslané pri zmene výberu.'], ['x: Int32', 'Vodorovná pozícia.'],
    ['y: Int32', 'Zvislá pozícia.'], ['width: Int32', 'Šírka zoznamu.'],
    ['height: Int32', 'Výška zoznamu.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'],
  ], 'Vytvorí zoznam. Položky pridaj cez `ui_list_item`.'),
  uiApi('ui_list_item', [
    ['event_id: Int32', 'ID zoznamu.'], ['label: String', 'Text novej položky.'],
  ], 'Pridá položku do existujúceho zoznamu; funguje aj po spustení okna.'),
  uiApi('ui_list_clear', [
    ['event_id: Int32', 'ID zoznamu.'],
  ], 'Vymaže všetky položky zoznamu a obnoví natívny ovládací prvok.'),
  uiApi('ui_list_count', [
    ['event_id: Int32', 'ID zoznamu.'],
  ], 'Vráti aktuálny počet položiek zoznamu.', 'Int32'),
  uiApi('ui_list_index', [
    ['event_id: Int32', 'ID zoznamu.'],
  ], 'Vráti index aktuálne vybranej položky od nuly.', 'Int32'),
  uiApi('ui_list_set_index', [
    ['event_id: Int32', 'ID zoznamu.'], ['selected_index: Int32', 'Index položky od nuly.'],
  ], 'Programovo vyberie položku zoznamu.'),
  uiApi('ui_list_set_item', [
    ['event_id: Int32', 'ID zoznamu.'], ['item_index: Int32', 'Index existujúcej položky od nuly.'],
    ['label: String', 'Nový text položky.'],
  ], 'Prepíše existujúcu položku a synchronizuje natívny zoznam.'),
  uiApi('ui_list_read_item', [
    ['event_id: Int32', 'ID zoznamu.'], ['item_index: Int32', 'Index existujúcej položky od nuly.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text položky.'],
  ], 'Skopíruje text položky do bounded UTF-8 buffra a vráti počet bajtov.', 'UIntSize'),
  uiApi('ui_list_bind_app', [
    ['event_id: Int32', 'ID natívneho zoznamu.'], ['list_id: Int32', 'ID bounded `app_list` kolekcie.'],
  ], 'Naviaže natívny zoznam na aplikačný zoznam a okamžite ho naplní.'),
  uiApi('ui_list_refresh_app', [
    ['event_id: Int32', 'ID natívneho zoznamu.'],
  ], 'Explicitne obnoví natívny zoznam z napojenej `app_list` kolekcie.'),
  uiApi('ui_table', [
    ['event_id: Int32', 'ID odoslané pri výbere riadku.'], ['x: Int32', 'Vodorovná pozícia.'],
    ['y: Int32', 'Zvislá pozícia.'], ['width: Int32', 'Šírka tabuľky.'],
    ['height: Int32', 'Výška tabuľky.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'],
  ], 'Vytvorí natívnu tabuľku v režime report view. Stĺpce pridaj cez `ui_table_column`.'),
  uiApi('ui_table_column', [
    ['event_id: Int32', 'ID tabuľky.'], ['column_index: Int32', 'Index stĺpca od nuly.'],
    ['title: String', 'Text hlavičky.'], ['width: Int32', 'Šírka stĺpca v pixeloch.'],
  ], 'Pridá alebo upraví hlavičku tabuľky.'),
  uiApi('ui_table_cell', [
    ['event_id: Int32', 'ID tabuľky.'], ['row_index: Int32', 'Index riadku od nuly.'],
    ['column_index: Int32', 'Index stĺpca od nuly.'], ['text: String', 'Text bunky.'],
  ], 'Nastaví bunku a vytvorí riadok, ak ešte neexistuje.'),
  uiApi('ui_table_read_cell', [
    ['event_id: Int32', 'ID tabuľky.'], ['row_index: Int32', 'Index riadku od nuly.'],
    ['column_index: Int32', 'Index stĺpca od nuly.'],
    ['output: write Slice<UInt8>', 'Caller-owned buffer pre UTF-8 text bunky.'],
  ], 'Skopíruje text bunky do bounded UTF-8 buffra a vráti počet bajtov.', 'UIntSize'),
  uiApi('ui_table_bind_app', [
    ['event_id: Int32', 'ID natívnej tabuľky.'], ['table_id: Int32', 'ID bounded `app_table` kolekcie.'],
    ['column_count: Int32', 'Počet zobrazovaných stĺpcov v rozsahu `1..8`.'],
  ], 'Naviaže natívnu tabuľku na aplikačnú tabuľku a načíta jej riadky.'),
  uiApi('ui_table_refresh_app', [
    ['event_id: Int32', 'ID natívnej tabuľky.'],
  ], 'Explicitne obnoví natívnu tabuľku z napojenej `app_table` kolekcie.'),
  uiApi('ui_refresh_bindings', [], 'Obnoví všetky väzby zoznamov, tabuliek a textových inputov naraz; po UI callbacku sa vo Windows preview volá automaticky.'),
  uiApi('ui_table_clear', [
    ['event_id: Int32', 'ID tabuľky.'],
  ], 'Vymaže všetky riadky tabuľky.'),
  uiApi('ui_table_row_count', [
    ['event_id: Int32', 'ID tabuľky.'],
  ], 'Vráti aktuálny počet riadkov tabuľky.', 'Int32'),
  uiApi('ui_table_selected_row', [
    ['event_id: Int32', 'ID tabuľky.'],
  ], 'Vráti index vybraného riadku alebo `-1`.', 'Int32'),
  uiApi('ui_table_set_selected_row', [
    ['event_id: Int32', 'ID tabuľky.'], ['row_index: Int32', 'Index riadku alebo `-1` na zrušenie.'],
  ], 'Programovo vyberie riadok tabuľky.'),
  uiApi('ui_table_sort_text', [
    ['event_id: Int32', 'ID tabuľky.'], ['column_index: Int32', 'Index textového stĺpca.'],
    ['descending: Bool', '`true` zostupne, `false` vzostupne.'],
  ], 'Zoradí napojenú `app_table` podľa textového stĺpca a obnoví tabuľku.'),
  uiApi('ui_table_sort_int', [
    ['event_id: Int32', 'ID tabuľky.'], ['column_index: Int32', 'Index signed integer stĺpca.'],
    ['descending: Bool', '`true` zostupne, `false` vzostupne.'],
  ], 'Zoradí napojenú `app_table` podľa typed Int stĺpca a obnoví tabuľku.'),
  uiApi('ui_table_sort_uint', [
    ['event_id: Int32', 'ID tabuľky.'], ['column_index: Int32', 'Index unsigned integer stĺpca.'],
    ['descending: Bool', '`true` zostupne, `false` vzostupne.'],
  ], 'Zoradí napojenú `app_table` podľa typed UInt stĺpca a obnoví tabuľku.'),
  uiApi('ui_table_sort_float', [
    ['event_id: Int32', 'ID tabuľky.'], ['column_index: Int32', 'Index Float stĺpca.'],
    ['descending: Bool', '`true` zostupne, `false` vzostupne.'],
  ], 'Zoradí napojenú `app_table` podľa typed Float stĺpca a obnoví tabuľku.'),
  uiApi('ui_table_sort_bool', [
    ['event_id: Int32', 'ID tabuľky.'], ['column_index: Int32', 'Index Bool stĺpca.'],
    ['descending: Bool', '`true` zostupne, `false` vzostupne.'],
  ], 'Zoradí napojenú `app_table` podľa typed Bool stĺpca a obnoví tabuľku.'),
  uiApi('ui_table_filter_text', [
    ['event_id: Int32', 'ID zdrojovej tabuľky.'], ['destination_table_id: Int32', 'Cieľová `app_table` kolekcia.'],
    ['column_index: Int32', 'Index textového stĺpca.'], ['query: String', 'Presná hodnota UTF-8.'],
  ], 'Skopíruje presné zhody do cieľovej `app_table` a obnoví napojené tabuľky.'),
  uiApi('ui_table_filter_text_ex', [
    ['event_id: Int32', 'ID zdrojovej tabuľky.'], ['destination_table_id: Int32', 'Cieľová `app_table` kolekcia.'],
    ['column_index: Int32', 'Index textového stĺpca.'], ['query: String', 'UTF-8 filter.'],
    ['mode: Int32', '`0` exact, `1` contains, `2` prefix, `3` suffix; `+4` ignoruje ASCII veľkosť.'],
  ], 'Rozšírený bounded filter napojenej tabuľky.'),
  uiApi('ui_table_filter_int', [
    ['event_id: Int32', 'ID zdrojovej tabuľky.'], ['destination_table_id: Int32', 'Cieľová `app_table` kolekcia.'],
    ['column_index: Int32', 'Index signed integer stĺpca.'], ['query: Int64', 'Presná signed hodnota.'],
  ], 'Skopíruje presné typed Int zhody do cieľovej `app_table` a obnoví napojené tabuľky.'),
  uiApi('ui_table_filter_uint', [
    ['event_id: Int32', 'ID zdrojovej tabuľky.'], ['destination_table_id: Int32', 'Cieľová `app_table` kolekcia.'],
    ['column_index: Int32', 'Index unsigned integer stĺpca.'], ['query: UInt64', 'Presná unsigned hodnota.'],
  ], 'Skopíruje presné typed UInt zhody do cieľovej `app_table` a obnoví napojené tabuľky.'),
  uiApi('ui_table_filter_float', [
    ['event_id: Int32', 'ID zdrojovej tabuľky.'], ['destination_table_id: Int32', 'Cieľová `app_table` kolekcia.'],
    ['column_index: Int32', 'Index Float stĺpca.'], ['query: Float64', 'Presná konečná hodnota.'],
  ], 'Skopíruje presné typed Float zhody do cieľovej `app_table` a obnoví napojené tabuľky.'),
  uiApi('ui_table_filter_bool', [
    ['event_id: Int32', 'ID zdrojovej tabuľky.'], ['destination_table_id: Int32', 'Cieľová `app_table` kolekcia.'],
    ['column_index: Int32', 'Index Bool stĺpca.'], ['query: Bool', 'Presná boolean hodnota.'],
  ], 'Skopíruje presné typed Bool zhody do cieľovej `app_table` a obnoví napojené tabuľky.'),
  uiApi('ui_tooltip', [
    ['event_id: Int32', 'ID eventového tlačidla, checkboxu alebo switchu.'],
    ['text: String', 'Text hover nápovedy.'], ['width: Int32', 'Maximálna šírka; Windows text prirodzene zalomí.'],
    ['height: Int32', 'Kompatibilitný parameter; výšku určuje natívny Windows tooltip.'], ['text_color: UInt32', 'Farba textu.'],
    ['background_color: UInt32', 'Farba pozadia.'], ['corner_radius: Int32', 'Kompatibilitný parameter; tvar určuje systémový tooltip.'],
  ], 'Priradí prirodzenú Windows hover nápovedu k existujúcemu eventovému prvku.'),
  uiApi('ui_theme', [
    ['mode: Int32', '`0` svetlá, `1` tmavá, `2` systémová (v preview svetlý fallback).'],
  ], 'Prepne semantickú tému UI počas behu.'),
  uiApi('ui_theme_color', [
    ['role: Int32', '`0` okno, `1` top bar, `2` panel, `3` primary, `4` safe, `5` status, `6` text, `7` secondary text.'],
  ], 'Vráti semantickú farbu aktuálnej témy.', 'UInt32'),
  uiApi('ui_image', [
    ['path: String', 'Relatívna literálna cesta k PNG alebo SVG v entry `.jdn` súbore.'],
    ['x: Int32', 'Vodorovná pozícia.'], ['y: Int32', 'Zvislá pozícia.'],
    ['width: Int32', 'Cieľová šírka.'], ['height: Int32', 'Cieľová výška.'],
  ], 'Vykreslí asset. Builder kopíruje PNG a SVG prerasterizuje do PNG vedľa `.exe`. '),
  uiApi('unity_editor_plan_copy_asset_path', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 bytes of a project-relative `Assets/...` prefab path.'],
    ['input_length: UIntSize', 'Valid byte count; it must not exceed the input slice capacity.'],
    ['output: write Slice<UInt8>', 'Caller-owned UTF-8 pool receiving the copied path.'],
    ['output_capacity: UIntSize', 'Usable capacity of the output pool.'],
  ], 'Copies a bounded prefab path into the caller-owned UTF-8 pool used by an Editor plan.', 'UIntSize'),
  uiApi('unity_editor_plan_copy_text_at', [
    ['input: read Slice<UInt8>', 'Caller-owned UTF-8 bytes for a UI label.'],
    ['input_length: UIntSize', 'Valid byte count; it must not exceed the input slice capacity.'],
    ['output: write Slice<UInt8>', 'Caller-owned UTF-8 pool receiving the copied label.'],
    ['output_capacity: UIntSize', 'Usable capacity of the output pool.'],
    ['output_offset: UIntSize', 'Destination offset in the pool.'],
  ], 'Copies a bounded UI label into a caller-owned UTF-8 pool at an explicit offset.', 'UIntSize'),
  uiApi('unity_editor_plan_bind_existing', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Explicit scene-registry handle of an existing object.'],
  ], 'Adds a bind-existing operation. Unity resolves only the explicit scene handle; no global search or reflection is used.', 'Bool'),
  uiApi('unity_editor_plan_create_primitive', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New object handle.'], ['primitive: UInt32', 'Unity primitive id: 0 Sphere, 1 Capsule, 2 Cylinder, 3 Cube, 4 Plane, 5 Quad.'],
    ['parent_id: UInt32', 'Explicit parent handle; `0` means scene root.'],
    ['transform: UnityTransform', 'Position, rotation, and scale.'], ['active: Bool', 'Initial active state.'],
  ], 'Adds a bounded primitive-creation operation to an Editor plan.', 'Bool'),
  uiApi('unity_editor_plan_instantiate_prefab', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New object handle.'], ['parent_id: UInt32', 'Explicit parent handle; `0` means scene root.'],
    ['asset_offset: UInt32', 'Offset of the prefab path in the caller-owned UTF-8 pool.'],
    ['asset_length: UInt32', 'Valid UTF-8 byte count of the prefab path.'],
    ['transform: UnityTransform', 'Position, rotation, and scale.'], ['active: Bool', 'Initial active state.'],
  ], 'Adds a validated project-prefab instantiation operation to an Editor plan.', 'Bool'),
  uiApi('unity_editor_plan_set_transform', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created or explicitly bound handle.'], ['transform: UnityTransform', 'New position, rotation, and scale.'],
  ], 'Adds a transform mutation for a previously created or explicitly bound object.', 'Bool'),
  uiApi('unity_editor_plan_set_active', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created or explicitly bound handle.'], ['active: Bool', 'New active state.'],
  ], 'Adds an active-state mutation for an explicit object handle.', 'Bool'),
  uiApi('unity_editor_plan_set_parent', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Object to reparent.'], ['parent_id: UInt32', 'Explicit parent handle; `0` detaches to the scene root.'],
  ], 'Adds a parent mutation for explicit object handles.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_canvas', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New Canvas handle.'], ['parent_id: UInt32', 'Explicit parent handle; `0` means scene root.'],
    ['active: Bool', 'Initial active state.'],
  ], 'Creates a ScreenSpaceOverlay UGUI Canvas with ScaleWithScreenSize configuration.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_panel', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New Image panel handle.'], ['parent_id: UInt32', 'Explicit Canvas or panel parent handle.'],
    ['position_x: Float32', 'Anchored X position.'], ['position_y: Float32', 'Anchored Y position.'],
    ['width: Float32', 'Positive panel width.'], ['height: Float32', 'Positive panel height.'],
    ['red: Float32', 'Color red channel in `0..1`.'], ['green: Float32', 'Color green channel in `0..1`.'],
    ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
    ['active: Bool', 'Initial active state.'],
  ], 'Creates a bounded UGUI Image panel with anchored position, size, and RGBA color.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_text', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New Text handle.'], ['parent_id: UInt32', 'Explicit parent handle.'],
    ['text_offset: UInt32', 'Offset of the label in the caller-owned UTF-8 pool.'], ['text_length: UInt32', 'Valid UTF-8 byte count, maximum 1024.'],
    ['font_size: UInt32', 'Font size in the bounded range `1..256`.'],
    ['position_x: Float32', 'Anchored X position.'], ['position_y: Float32', 'Anchored Y position.'],
    ['width: Float32', 'Positive text width.'], ['height: Float32', 'Positive text height.'],
    ['red: Float32', 'Color red channel in `0..1`.'], ['green: Float32', 'Color green channel in `0..1`.'],
    ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
    ['active: Bool', 'Initial active state.'],
  ], 'Creates a bounded UGUI Text element from a caller-owned UTF-8 label.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_button', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New Button handle.'], ['parent_id: UInt32', 'Explicit parent handle.'],
    ['text_offset: UInt32', 'Offset of the button label in the caller-owned UTF-8 pool.'], ['text_length: UInt32', 'Valid UTF-8 byte count, maximum 1024.'],
    ['font_size: UInt32', 'Label font size in the bounded range `1..256`.'],
    ['position_x: Float32', 'Anchored X position.'], ['position_y: Float32', 'Anchored Y position.'],
    ['width: Float32', 'Positive button width.'], ['height: Float32', 'Positive button height.'],
    ['red: Float32', 'Color red channel in `0..1`.'], ['green: Float32', 'Color green channel in `0..1`.'],
    ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
    ['active: Bool', 'Initial active state.'],
  ], 'Creates a bounded UGUI Button with a child Text label.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_toggle', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New Toggle handle.'], ['parent_id: UInt32', 'Explicit parent handle.'],
    ['text_offset: UInt32', 'Offset of the toggle label in the caller-owned UTF-8 pool.'], ['text_length: UInt32', 'Valid UTF-8 byte count, maximum 1024.'],
    ['font_size: UInt32', 'Label font size in the bounded range `1..256`.'],
    ['position_x: Float32', 'Anchored X position.'], ['position_y: Float32', 'Anchored Y position.'],
    ['width: Float32', 'Positive toggle width.'], ['height: Float32', 'Positive toggle height.'],
    ['red: Float32', 'Color red channel in `0..1`.'], ['green: Float32', 'Color green channel in `0..1`.'],
    ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
    ['initial_checked: Bool', 'Initial checked state.'], ['active: Bool', 'Initial active state.'],
  ], 'Creates a bounded UGUI Toggle with a child Text label.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_slider', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New Slider handle.'], ['parent_id: UInt32', 'Explicit parent handle.'],
    ['initial_value: Float32', 'Normalized initial value in `0..1`.'],
    ['position_x: Float32', 'Anchored X position.'], ['position_y: Float32', 'Anchored Y position.'],
    ['width: Float32', 'Positive slider width.'], ['height: Float32', 'Positive slider height.'],
    ['red: Float32', 'Color red channel in `0..1`.'], ['green: Float32', 'Color green channel in `0..1`.'],
    ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
    ['active: Bool', 'Initial active state.'],
  ], 'Creates a bounded UGUI Slider with a normalized initial value.', 'Bool'),
  uiApi('unity_editor_plan_create_ui_input_field', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'New InputField handle.'], ['parent_id: UInt32', 'Explicit parent handle.'],
    ['text_offset: UInt32', 'Offset of the initial text in the caller-owned UTF-8 pool.'], ['text_length: UInt32', 'Valid UTF-8 byte count, maximum 1024.'],
    ['font_size: UInt32', 'Text font size in the bounded range `1..256`.'],
    ['position_x: Float32', 'Anchored X position.'], ['position_y: Float32', 'Anchored Y position.'],
    ['width: Float32', 'Positive input width.'], ['height: Float32', 'Positive input height.'],
    ['red: Float32', 'Color red channel in `0..1`.'], ['green: Float32', 'Color green channel in `0..1`.'],
    ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
    ['active: Bool', 'Initial active state.'],
  ], 'Creates a bounded UGUI InputField with editable initial text.', 'Bool'),
  uiApi('unity_editor_plan_attach_ui_text_event_source', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Known legacy UGUI InputField handle.'], ['router_id: UInt32', 'Explicit Jadren UI event router handle.'],
  ], 'Attaches the explicit UTF-8 InputField event adapter to a Jadren UI event router.', 'Bool'),
  uiApi('unity_editor_plan_set_ui_text', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created Text or Button handle.'],
    ['text_offset: UInt32', 'Offset of the new label in the caller-owned UTF-8 pool.'], ['text_length: UInt32', 'Valid UTF-8 byte count, maximum 1024.'],
  ], 'Changes the label on a previously created Text or Button.', 'Bool'),
  uiApi('unity_editor_plan_set_ui_color', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created UI handle.'], ['red: Float32', 'Color red channel in `0..1`.'],
    ['green: Float32', 'Color green channel in `0..1`.'], ['blue: Float32', 'Color blue channel in `0..1`.'], ['alpha: Float32', 'Color alpha channel in `0..1`.'],
  ], 'Changes the first UGUI Graphic color on an explicit UI handle.', 'Bool'),
  uiApi('unity_editor_plan_set_ui_interactable', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created Button handle.'], ['interactable: Bool', 'Whether the Button accepts interaction.'],
  ], 'Enables or disables interaction on an explicit UGUI Button handle.', 'Bool'),
  uiApi('unity_editor_plan_add_component', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created or explicitly bound object handle.'],
    ['component_kind: UInt32', '`1` Rigidbody, `2` BoxCollider.'],
  ], 'Adds one allowlisted Unity component to an explicit object handle.', 'Bool'),
  uiApi('unity_editor_plan_set_component_float', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created or explicitly bound object handle.'],
    ['component_kind: UInt32', '`1` Rigidbody, `2` BoxCollider.'],
    ['property_kind: UInt32', '`1` means Rigidbody.mass or BoxCollider.contactOffset.'],
    ['value: Float32', 'Finite non-negative property value.'],
  ], 'Sets one allowlisted Float32 property on a Unity component.', 'Bool'),
  uiApi('unity_editor_plan_set_component_bool', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created or explicitly bound object handle.'],
    ['component_kind: UInt32', '`1` Rigidbody, `2` BoxCollider.'],
    ['property_kind: UInt32', 'Rigidbody: `1` useGravity, `2` isKinematic; BoxCollider: `1` isTrigger.'],
    ['value: Bool', 'New property value.'],
  ], 'Sets one allowlisted Bool property on a Unity component.', 'Bool'),
  uiApi('unity_editor_plan_save_prefab_asset', [
    ['output: write Slice<UnityEditorPlanCommandRecord>', 'Caller-owned fixed-size Editor plan records.'],
    ['capacity: UIntSize', 'Record capacity.'], ['slot: UIntSize', 'Record slot to write.'],
    ['target_id: UInt32', 'Previously created or explicitly bound object handle.'],
    ['asset_offset: UInt32', 'Offset of the destination path in the caller-owned UTF-8 pool.'],
    ['asset_length: UInt32', 'Bounded byte length of the Assets/*.prefab destination.'],
  ], 'Saves an explicit scene object as a new project-relative prefab asset during Editor Apply.', 'Bool'),
  uiApi('ui_run', [], 'Spustí UI message loop. Vráť jeho hodnotu z `main`.', 'Int32'),
];

const UI_API_BY_NAME = new Map(UI_API.map((api) => [api.name, api]));

const ENGLISH_UI_DOCS = new Map([
  ['ui_window', 'Creates the main native Windows window. Call it before other UI elements.'],
  ['ui_app_begin', 'Starts the retained UI tree and returns its root node id. The source contract is platform-neutral.'],
  ['ui_app_on_resize', 'Registers an explicit event ID delivered after native window layout resize.'],
  ['ui_app_on_close', 'Registers an explicit event ID delivered once before native window close.'],
  ['ui_app_window_width', 'Returns the current native client width at callback time.'],
  ['ui_app_window_height', 'Returns the current native client height at callback time.'],
  ['ui_app_panel', 'Adds a nested retained panel and returns its node id. Close it with ui_app_end.'],
  ['ui_app_row', 'Adds a horizontal retained layout scope and returns its node id. Close it with ui_app_end.'],
  ['ui_app_top_bar', 'Adds a horizontal retained top bar with background styling and returns its node id.'],
  ['ui_app_menu', 'Adds a retained menu trigger whose popup is populated with ui_app_menu_item.'],
  ['ui_app_menu_item', 'Adds one item to the previously declared retained popup menu.'],
  ['ui_app_label', 'Adds a label to the current retained panel and returns its node id.'],
  ['ui_app_status', 'Adds a retained status label updated by ui_set_status.'],
  ['ui_app_button', 'Adds an event button to the current retained panel and returns its node id.'],
  ['ui_app_tooltip', 'Attaches a bounded hover tooltip to a retained event button or checkbox.'],
  ['ui_app_text_input', 'Adds a retained single-line text input to the current panel and returns its node id.'],
  ['ui_app_checkbox', 'Adds a retained checkbox to the current panel and returns its node id.'],
  ['ui_app_select', 'Adds a retained dropdown to the current panel and returns its node id.'],
  ['ui_app_select_option', 'Adds an option to a retained dropdown.'],
  ['ui_app_select_index', 'Reads the selected option index from a retained dropdown.'],
  ['ui_app_select_set_index', 'Sets the selected option index on a retained dropdown.'],
  ['ui_app_list', 'Adds a retained dynamic list to the current panel.'],
  ['ui_app_list_item', 'Appends an item to a retained dynamic list.'],
  ['ui_app_list_clear', 'Clears a retained dynamic list.'],
  ['ui_app_list_count', 'Reads the item count of a retained dynamic list.'],
  ['ui_app_list_index', 'Reads the selected index of a retained dynamic list.'],
  ['ui_app_list_set_index', 'Sets the selected index of a retained dynamic list.'],
  ['ui_app_list_bind_app', 'Binds a retained dynamic list to app_list state.'],
  ['ui_app_list_refresh', 'Refreshes a retained dynamic list from app_list state.'],
  ['ui_app_table', 'Adds a retained report table to the current panel.'],
  ['ui_app_table_column', 'Declares a retained table column.'],
  ['ui_app_table_cell', 'Writes a retained table cell.'],
  ['ui_app_table_read_cell', 'Reads a retained table cell into a caller-owned buffer.'],
  ['ui_app_table_bind_app', 'Binds a retained table to app_table state.'],
  ['ui_app_table_refresh', 'Refreshes a retained table from app_table state.'],
  ['ui_app_table_clear', 'Clears a retained table.'],
  ['ui_app_table_row_count', 'Reads the row count of a retained table.'],
  ['ui_app_table_selected_row', 'Reads the selected row of a retained table.'],
  ['ui_app_table_set_selected_row', 'Sets the selected row of a retained table.'],
  ['ui_app_table_sort_text', 'Sorts a bound app_table by a text column.'],
  ['ui_app_table_sort_int', 'Sorts a bound app_table by a typed signed integer column.'],
  ['ui_app_table_sort_uint', 'Sorts a bound app_table by a typed unsigned integer column.'],
  ['ui_app_table_sort_float', 'Sorts a bound app_table by a typed float column.'],
  ['ui_app_table_sort_bool', 'Sorts a bound app_table by a typed boolean column.'],
  ['ui_app_table_filter_text', 'Copies exact text matches into another app_table.'],
  ['ui_app_table_filter_text_ex', 'Filters a bound app_table with an explicit bounded match mode.'],
  ['ui_app_table_filter_int', 'Copies exact typed signed integer matches into another app_table.'],
  ['ui_app_table_filter_uint', 'Copies exact typed unsigned integer matches into another app_table.'],
  ['ui_app_table_filter_float', 'Copies exact typed float matches into another app_table.'],
  ['ui_app_table_filter_bool', 'Copies exact typed boolean matches into another app_table.'],
  ['ui_list_bind_app_state', 'Binds a native list selection to an Int64 app_state index.'],
  ['ui_list_refresh_app_state', 'Refreshes a native list selection from its Int64 app_state index.'],
  ['ui_table_bind_app_state', 'Binds a native table row selection to an Int64 app_state index.'],
  ['ui_table_refresh_app_state', 'Refreshes a native table row selection from its Int64 app_state index.'],
  ['ui_table_sort_text', 'Sorts a bound app_table by a text column from an event callback.'],
  ['ui_table_sort_int', 'Sorts a bound app_table by a typed signed integer column from an event callback.'],
  ['ui_table_sort_uint', 'Sorts a bound app_table by a typed unsigned integer column from an event callback.'],
  ['ui_table_sort_float', 'Sorts a bound app_table by a typed float column from an event callback.'],
  ['ui_table_sort_bool', 'Sorts a bound app_table by a typed boolean column from an event callback.'],
  ['ui_table_filter_text', 'Copies exact text matches into another app_table from an event callback.'],
  ['ui_table_filter_text_ex', 'Filters a bound app_table with a bounded match mode from an event callback.'],
  ['ui_table_filter_int', 'Copies exact typed signed integer matches into another app_table from an event callback.'],
  ['ui_table_filter_uint', 'Copies exact typed unsigned integer matches into another app_table from an event callback.'],
  ['ui_table_filter_float', 'Copies exact typed float matches into another app_table from an event callback.'],
  ['ui_table_filter_bool', 'Copies exact typed boolean matches into another app_table from an event callback.'],
  ['ui_app_bind_app_state', 'Binds a retained text input, checkbox, dropdown, list, or table to app_state by node id.'],
  ['ui_app_refresh_app_state', 'Refreshes a retained value control from app_state by node id.'],
  ['ui_app_end', 'Closes the most recently opened retained root or panel scope and returns whether the order was valid.'],
  ['ui_app_run', 'Validates the retained UI tree and starts the active platform message loop.'],
  ['buffer_resize_move', 'Resizes an owning nested buffer or direct @repr(C) record Buffer with zero-initialized growth and shrink cleanup.'],
  ['buffer_resize_move_status', 'Returns a bounded status code for an owning nested or record-buffer resize.'],
  ['ui_top_bar', 'Creates a top bar that stretches to the window width when the window is resized.'],
  ['ui_button', 'Creates a button with hover, pressed, and focus feedback.'],
  ['ui_toggle_button', 'Creates a toggle button that keeps its selected state.'],
  ['ui_disabled_button', 'Creates a disabled button without a clickable action.'],
  ['ui_close_button', 'Creates a button that closes the window.'],
  ['ui_icon_button', 'Creates an icon button anchored to the right edge. Use a system icon name or Unicode text.'],
  ['ui_menu', 'Creates a native Windows drop-down menu. Declare ui_menu_option immediately after it.'],
  ['ui_menu_option', 'Adds an item to the previously declared ui_menu.'],
  ['ui_tooltip', 'Attaches a native Windows hover tooltip to an existing interactive control.'],
  ['ui_refresh_bindings', 'Refreshes all bound lists, tables, and text inputs in one pass. The Windows preview also runs it automatically after a UI callback.'],
  ['ui_theme', 'Selects the active semantic theme: 0 light, 1 dark, or 2 system fallback.'],
  ['ui_theme_color', 'Returns a semantic color from the active theme. It is a getter; use a 0xRRGGBBu32 literal for a custom color.'],
  ['ui_image', 'Draws a project-relative PNG or SVG asset.'],
  ['unity_editor_plan_copy_asset_path', 'Copies a bounded project-relative prefab path into the caller-owned UTF-8 pool used by an Editor plan.'],
  ['unity_editor_plan_copy_text_at', 'Copies a bounded UI label into a caller-owned UTF-8 pool at an explicit offset.'],
  ['unity_editor_plan_bind_existing', 'Adds a bind-existing operation. Unity resolves only the explicit scene-registry handle; no global search or reflection is used.'],
  ['unity_editor_plan_create_primitive', 'Adds a bounded primitive-creation operation to an Editor plan.'],
  ['unity_editor_plan_instantiate_prefab', 'Adds a validated project-prefab instantiation operation to an Editor plan.'],
  ['unity_editor_plan_set_transform', 'Adds a transform mutation for a previously created or explicitly bound object.'],
  ['unity_editor_plan_set_active', 'Adds an active-state mutation for an explicit object handle.'],
  ['unity_editor_plan_set_parent', 'Adds a parent mutation for explicit object handles.'],
  ['unity_editor_plan_create_ui_canvas', 'Creates a ScreenSpaceOverlay UGUI Canvas with ScaleWithScreenSize configuration.'],
  ['unity_editor_plan_create_ui_panel', 'Creates a bounded UGUI Image panel with anchored position, size, and RGBA color.'],
  ['unity_editor_plan_create_ui_text', 'Creates a bounded UGUI Text element from a caller-owned UTF-8 label.'],
  ['unity_editor_plan_create_ui_button', 'Creates a bounded UGUI Button with a child Text label.'],
  ['unity_editor_plan_create_ui_toggle', 'Creates a bounded UGUI Toggle with a child Text label.'],
  ['unity_editor_plan_create_ui_slider', 'Creates a bounded UGUI Slider with a normalized initial value.'],
  ['unity_editor_plan_create_ui_input_field', 'Creates a bounded UGUI InputField with editable initial text.'],
  ['unity_editor_plan_attach_ui_text_event_source', 'Attaches the explicit UTF-8 InputField event adapter to a Jadren UI event router.'],
  ['unity_editor_plan_set_ui_text', 'Changes the label on a previously created Text or Button.'],
  ['unity_editor_plan_set_ui_color', 'Changes the first UGUI Graphic color on an explicit UI handle.'],
  ['unity_editor_plan_set_ui_interactable', 'Enables or disables interaction on an explicit UGUI Button handle.'],
  ['unity_editor_plan_add_component', 'Adds one allowlisted Unity component to an explicit object handle.'],
  ['unity_editor_plan_set_component_float', 'Sets one allowlisted Float32 property on a Unity component.'],
  ['unity_editor_plan_set_component_bool', 'Sets one allowlisted Bool property on a Unity component.'],
  ['unity_editor_plan_save_prefab_asset', 'Saves one explicit scene object as a new project-relative prefab asset during Editor Apply.'],
  ['app_table_set_cell_bytes', 'Writes one bounded table cell directly from a caller-owned UTF-8 byte slice.'],
  ['app_table_set_cell_bytes_ex', 'Writes only the explicit valid prefix of a caller-owned UTF-8 byte slice into one bounded table cell.'],
  ['app_table_filter_text_ex_bytes', 'Filters a bounded table directly from a caller-owned UTF-8 byte slice and an explicit valid length.'],
  ['app_table_filter_callback', 'Projects rows selected by a bounded read-only fn(Int32, Int32) -> Bool predicate into another app_table.'],
  ['app_table_sort_callback', 'Sorts a bounded app_table with a read-only fn(Int32, Int32, Int32) -> Int32 comparator and stable order.'],
  ['app_table_page', 'Projects one bounded app_table page in source row order while preserving its schema.'],
  ['app_list_filter_callback', 'Projects a bounded list with a read-only fn(Int32, Int32) -> Bool predicate and transactional destination publication.'],
  ['app_list_sort_callback', 'Sorts a bounded list with a read-only fn(Int32, Int32, Int32) -> Int32 comparator and stable order.'],
  ['app_list_page', 'Projects one bounded list page in source order with transactional destination publication.'],
  ['app_list_filter_text_ex_bytes', 'Filters a bounded list directly from a caller-owned UTF-8 byte slice and an explicit valid length.'],
  ['app_list_push_text_bytes', 'Appends a bounded list item directly from a caller-owned UTF-8 byte slice.'],
  ['app_list_set_text_bytes', 'Replaces a bounded list item directly from a caller-owned UTF-8 byte slice.'],
  ['app_list_export_csv', 'Exports a bounded text list as one-column CSV into a caller-owned byte slice.'],
  ['app_table_index_build', 'Builds a bounded deterministic index using an app_table column declared type.'],
  ['app_table_index_build_int', 'Builds a bounded numeric Int index for an app_table column.'],
  ['app_table_index_build_uint', 'Builds a bounded numeric UInt index for an app_table column.'],
  ['app_table_index_build_float', 'Builds a bounded numeric Float index for an app_table column.'],
  ['app_table_index_build_bool', 'Builds a bounded Bool index for an app_table column.'],
  ['app_table_index_build_pair', 'Builds a bounded deterministic lexicographic index over two text columns.'],
  ['app_table_index_clear', 'Clears an app_table index without changing table rows.'],
  ['app_table_index_find_text', 'Finds an exact value through a valid app_table text index.'],
  ['app_table_index_find_pair_text', 'Finds the first row matching two exact text keys through a valid pair index.'],
  ['app_table_index_find_int', 'Finds an exact Int value through a valid app_table index.'],
  ['app_table_index_collect_int_range', 'Collects row ids for an inclusive Int range through a valid app_table index.'],
  ['app_table_index_collect_uint_range', 'Collects row ids for an inclusive UInt range through a valid app_table index.'],
  ['app_table_index_collect_float_range', 'Collects row ids for an inclusive finite Float range through a valid app_table index.'],
  ['app_table_index_find_uint', 'Finds an exact UInt value through a valid app_table index.'],
  ['app_table_index_find_float', 'Finds an exact Float value through a valid app_table index.'],
  ['app_table_index_find_bool', 'Finds an exact Bool value through a valid app_table index.'],
  ['app_state_read_int', 'Reads an exact Int64 app_state value into output[0] and returns false on missing or type mismatch.'],
  ['app_state_read_uint', 'Reads an exact UInt64 app_state value into output[0] and returns false on missing or type mismatch.'],
  ['app_state_read_float', 'Reads an exact Float64 app_state value into output[0] and returns false on missing or type mismatch.'],
  ['app_state_read_bool', 'Reads an exact Bool app_state value into output[0] and returns false on missing or type mismatch.'],
  ['app_table_index_is_valid', 'Checks whether an app_table index still matches current table data.'],
  ['app_table_export_csv', 'Exports a bounded app_table as CSV into a caller-owned byte slice.'],
  ['app_table_import_csv', 'Imports standard bounded CSV from a caller-owned byte slice transactionally.'],
  ['ui_run', 'Starts the native UI message loop. Return its result from main.'],
  ['http_request_write', 'Builds a bounded HTTP request without performing socket or TLS I/O.'],
  ['http_request_write_prefix', 'Builds a bounded HTTP request from an explicit valid body prefix and capacity.'],
  ['http_request_write_header_block', 'Builds a bounded HTTP request with multiple validated CRLF-separated custom headers.'],
  ['bearer_token_matches', 'Compares an exact Bearer token without storing authentication state.'],
  ['cookie_value_matches', 'Matches one exact Cookie name=value pair without allocation or session state.'],
  ['http_response_write_cookie', 'Serializes one validated dynamic Set-Cookie response header without partial writes.'],
  ['http_response_write_cookie_ex', 'Serializes a validated dynamic Set-Cookie header with explicit keep-alive mode.'],
]);

// The UI catalog predates the language switch and contains a mixture of
// English and Slovak descriptions. Keep the most frequently used calls
// explicitly translated and use a safe Slovak fallback for the remaining
// English-only entries so `documentationLanguage = sk` never silently shows
// an English help card.
const SLOVAK_UI_DOCS = new Map([
  ['process_arg_count', 'Vráti počet natívnych argumentov procesu vrátane cesty k executable na indexe 0.'],
  ['process_arg_read', 'Skopíruje argument procesu ako UTF-8 do caller-owned buffera.'],
  ['stdin_read', 'Načíta najviac kapacitu výstupného buffera zo štandardného vstupu.'],
  ['stdout_write', 'Zapíše vstupný byte slice na štandardný výstup.'],
  ['stderr_write', 'Zapíše vstupný byte slice na chybový výstup.'],
  ['buffer_clear', 'Vymaže logickú dĺžku Buffer<T> bez uvoľnenia rezervovanej kapacity.'],
  ['buffer_clear_status', 'Statusová verzia vymazania Buffer<T>; vráti 0 pri úspechu.'],
  ['buffer_reserve', 'Zväčší minimálnu kapacitu generického Buffer<T>.'],
  ['buffer_resize_move', 'Zmení dĺžku owning bufferu a bezpečne spracuje rast aj zmenšenie.'],
  ['buffer_resize_move_status', 'Statusová verzia owning resize; vráti 0 pri úspechu.'],
  ['buffer_append_move', 'Presunie move-safe vlastnícku hodnotu na koniec Buffer<T> a po úspechu vynuluje zdroj.'],
  ['buffer_append_move_status', 'Statusová verzia move appendu; pri chybe nemení zdroj ani cieľový buffer.'],
  ['buffer_insert_move', 'Presunie owning hodnotu na zadanú pozíciu a posunie ďalšie prvky.'],
  ['buffer_remove_move', 'Presunie owning hodnotu zo zadanej pozície a vráti Result<T, Int32>.'],
  ['buffer_pop', 'Presunie posledný owning prvok von a vráti Result<T, Int32>.'],
  ['ui_app_begin', 'Spustí retained UI strom a vráti ID koreňového uzla.'],
  ['ui_app_end', 'Ukončí naposledy otvorený retained root alebo panel.'],
  ['ui_app_run', 'Overí retained UI strom a spustí natívnu message loop.'],
  ['ui_app_panel', 'Pridá vnorený retained panel a vráti jeho ID uzla.'],
  ['ui_app_row', 'Pridá horizontálny retained layout a vráti jeho ID uzla.'],
  ['ui_app_button', 'Pridá tlačidlo s udalosťou do aktuálneho retained panela.'],
  ['ui_app_tooltip', 'Pripojí tooltip pri hoveri k retained tlačidlu alebo checkboxu.'],
  ['ui_app_text_input', 'Pridá jednoriadkový textový vstup do aktuálneho panela.'],
  ['ui_app_checkbox', 'Pridá checkbox do aktuálneho panela.'],
  ['ui_app_select', 'Pridá rozbaľovací výber do aktuálneho panela.'],
  ['ui_app_list', 'Pridá dynamický retained zoznam do aktuálneho panela.'],
  ['ui_app_table', 'Pridá retained reportovaciu tabuľku do aktuálneho panela.'],
  ['ui_top_bar', 'Vytvorí horný panel, ktorý sa pri zmene veľkosti roztiahne na šírku okna.'],
  ['ui_button', 'Vytvorí tlačidlo s efektom hover, stlačenia a focusu.'],
  ['ui_toggle_button', 'Vytvorí tlačidlo, ktoré si udržiava vybraný stav.'],
  ['ui_disabled_button', 'Vytvorí neaktívne tlačidlo bez klikateľnej akcie.'],
  ['ui_close_button', 'Vytvorí tlačidlo na zatvorenie okna.'],
  ['ui_icon_button', 'Vytvorí ikonové tlačidlo ukotvené pri pravom okraji.'],
  ['ui_menu', 'Vytvorí natívne rozbaľovacie menu Windows.'],
  ['ui_menu_option', 'Pridá položku do naposledy deklarovaného menu.'],
  ['ui_tooltip', 'Pripojí natívny Windows tooltip k existujúcemu interaktívnemu prvku.'],
  ['ui_theme', 'Vyberie aktívnu sémantickú tému: 0 svetlá, 1 tmavá alebo 2 systémová.'],
  ['ui_theme_color', 'Vráti sémantickú farbu z aktívnej témy. Vlastnú farbu zapíš ako 0xRRGGBBu32.'],
  ['ui_image', 'Vykreslí projektový PNG alebo SVG asset.'],
  ['ui_run', 'Spustí natívnu UI message loop; výsledok vráť z main.'],
  ['http_request_write_header_block', 'Zostaví bounded HTTP request s viacerými validovanými CRLF hlavičkami; framing hlavičky a chybné riadky odmietne bez partial write.'],
  ['bearer_token_matches', 'Porovná presný Bearer token bez uloženia auth stavu.'],
  ['cookie_value_matches', 'Nájde jeden presný Cookie name=value pár bez alokácie alebo session stavu.'],
  ['http_response_write_cookie', 'Zostaví jednu validovanú dynamickú Set-Cookie response hlavičku bez partial write.'],
  ['http_response_write_cookie_ex', 'Zostaví dynamickú Set-Cookie hlavičku s explicitným keep-alive režimom.'],
  ['http_response_body_chunked_exact', '**http_response_body_chunked_exact** dekóduje kompletnú odpoveď `Transfer-Encoding: chunked`, overí trailer hlavičky, samostatne zapíše dĺžku a pri chybe nevykoná čiastočný zápis.'],
  ['http_request_body_chunked_exact', '**http_request_body_chunked_exact** dekóduje kompletnú požiadavku `Transfer-Encoding: chunked`, overí trailer hlavičky, samostatne zapíše dĺžku a pri chybe nevykoná čiastočný zápis.'],
  ['http_request_chunked_frame_length_prefix', 'Vráti presnú hranicu prvého kompletného chunked requestu z prijatého prefixu; 0 znamená neplatný alebo neúplný frame.'],
]);

function looksEnglishDocumentation(text) {
  return /\b(?:Creates?|Adds?|Attaches?|Builds?|Changes?|Clears?|Copies?|Draws?|Enables?|Filters?|Finds?|Reads?|Refreshes?|Returns?|Saves?|Selects?|Sets?|Starts?|Validates?|Writes?|Parameter|Caller-owned|Status form|New |Initial |Whether )\b/i.test(text || '');
}

function documentationLanguage(document) {
  const configuration = vscode.workspace.getConfiguration('jadren', document?.uri);
  return configuration.get('documentationLanguage', 'en') === 'sk' ? 'sk' : 'en';
}

function uiApiDocumentation(api, document) {
  if (documentationLanguage(document) === 'sk') {
    return SLOVAK_UI_DOCS.get(api.name)
      || (looksEnglishDocumentation(api.documentation)
        ? `Vstavané volanie Jadren **${api.name}**. Podrobnosti určuje jeho typový podpis.`
        : api.documentation);
  }
  return ENGLISH_UI_DOCS.get(api.name) || `Jadren built-in UI call: ${api.name}.`;
}

function uiApiParameterDocumentation(parameter, document) {
  if (documentationLanguage(document) === 'sk') {
    if (!looksEnglishDocumentation(parameter.documentation)) {
      return parameter.documentation;
    }
    const parts = parameter.label.split(':');
    return `Parameter \`${parts[0].trim()}\` typu \`${parts.slice(1).join(':').trim() || 'neznámy typ'}\`.`;
  }
  const parts = parameter.label.split(':');
  return `Parameter ${parts[0].trim()} has type ${parts.slice(1).join(':').trim()}.`;
}

function uiApiSignature(api) {
  return `${api.name}(${api.parameters.map((parameter) => parameter.label).join(', ')}) -> ${api.returnType}`;
}

function uiApiSnippet(api) {
  if (api.parameters.length === 0) {
    return `${api.name}()`;
  }
  return `${api.name}(${api.parameters.map((parameter, index) => `\${${index + 1}:${parameter.label.split(':')[0].trim()}}`).join(', ')})`;
}

function uiApiMarkdown(api, document) {
  return new vscode.MarkdownString(
    `**${api.name}**\n\n${uiApiDocumentation(api, document)}\n\n\`\`\`jadren\n${uiApiSignature(api)}\n\`\`\`\n\n${documentationLanguage(document) === 'sk' ? 'Vracia' : 'Returns'}: \`${api.returnType}\``,
  );
}

// The language server supplies the authoritative, type-checked symbol list.
// Keep a small source-local fallback here as well so completion remains useful
// while a file is being edited, has a temporary syntax error, or the LSP is
// still starting. This deliberately reads only the current document and never
// guesses symbols from another project.
function maskJadrenSource(source) {
  let result = '';
  let quote = '';
  let escaped = false;
  let lineComment = false;
  let blockComment = false;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (lineComment) {
      result += character === '\n' ? '\n' : ' ';
      if (character === '\n') {
        lineComment = false;
      }
      continue;
    }
    if (blockComment) {
      result += character === '\n' ? '\n' : ' ';
      if (character === '*' && next === '/') {
        result += ' ';
        index += 1;
        blockComment = false;
      }
      continue;
    }
    if (quote) {
      result += character === '\n' ? '\n' : ' ';
      if (escaped) {
        escaped = false;
      } else if (character === '\\') {
        escaped = true;
      } else if (character === quote) {
        quote = '';
      }
      continue;
    }
    if (character === '/' && next === '/') {
      result += '  ';
      index += 1;
      lineComment = true;
      continue;
    }
    if (character === '/' && next === '*') {
      result += '  ';
      index += 1;
      blockComment = true;
      continue;
    }
    if (character === '"' || character === "'") {
      result += ' ';
      quote = character;
      continue;
    }
    result += character;
  }
  return result;
}

function localSymbolsForCompletion(document, position) {
  const source = document.getText().slice(0, document.offsetAt(position));
  const masked = maskJadrenSource(source);
  const slovak = documentationLanguage(document) === 'sk';
  const symbols = new Map();
  const add = (name, kind, detail, documentation) => {
    if (!name || symbols.has(name)) {
      return;
    }
    symbols.set(name, { name, kind, detail, documentation });
  };

  const functionPattern = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)\s*(?:->\s*([^\s{]+))?/g;
  let match;
  while ((match = functionPattern.exec(masked)) !== null) {
    const returnType = match[3] ? ` -> ${match[3]}` : '';
    add(
      match[1],
      vscode.CompletionItemKind.Function,
      `fn ${match[1]}(${match[2]})${returnType}`,
      `${slovak ? 'Lokálna funkcia deklarovaná v tomto súbore Jadren.' : 'Local function declared in this Jadren file.'}\n\n\`\`\`jadren\nfn ${match[1]}(${match[2]})${returnType} { ... }\n\`\`\``
    );
    for (const parameter of match[2].split(',')) {
      const parameterMatch = parameter.trim().match(/^([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*(.+))?$/);
      if (!parameterMatch) {
        continue;
      }
      const parameterType = parameterMatch[2] ? `: ${parameterMatch[2].trim()}` : '';
      add(
        parameterMatch[1],
        vscode.CompletionItemKind.Variable,
        `parameter ${parameterMatch[1]}${parameterType}`,
        `${slovak ? 'Parameter funkcie deklarovaný v tomto súbore Jadren.' : 'Function parameter declared in this Jadren file.'}${parameterType ? `\n\n${slovak ? 'Typ' : 'Type'}: \`${parameterMatch[2].trim()}\`` : ''}`
      );
    }
  }

  const bindingPattern = /\b(let|var|const)\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*([^=;\n{}]+))?/g;
  while ((match = bindingPattern.exec(masked)) !== null) {
    const type = match[3] ? match[3].trim() : '';
    add(
      match[2],
      vscode.CompletionItemKind.Variable,
      `${match[1]} ${match[2]}${type ? `: ${type}` : ''}`,
      `${slovak ? `${match[1]} väzba deklarovaná v tomto súbore Jadren.` : `${match[1]} binding declared in this Jadren file.`}${type ? `\n\n${slovak ? 'Typ' : 'Type'}: \`${type}\`` : ''}`
    );
  }

  const typePattern = /\b(struct|enum|trait|type)\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  while ((match = typePattern.exec(masked)) !== null) {
    add(
      match[2],
      vscode.CompletionItemKind.TypeParameter,
      `${match[1]} ${match[2]}`,
      slovak ? `${match[1]} deklarovaný v tomto súbore Jadren.` : `${match[1]} declared in this Jadren file.`
    );
  }

  return [...symbols.values()];
}

function registerColorProvider(context) {
  const provider = vscode.languages.registerColorProvider(
    { scheme: 'file', language: 'jadren' },
    {
      provideDocumentColors(document) {
        const colors = [];
        const source = document.getText();
        const pattern = /0x([0-9A-Fa-f]{6})u32\b/g;
        let match;
        while ((match = pattern.exec(source)) !== null) {
          const digits = match[1];
          const start = document.positionAt(match.index);
          const end = document.positionAt(match.index + match[0].length);
          colors.push({
            color: new vscode.Color(
              parseInt(digits.slice(0, 2), 16) / 255,
              parseInt(digits.slice(2, 4), 16) / 255,
              parseInt(digits.slice(4, 6), 16) / 255,
              1,
            ),
            range: new vscode.Range(start, end),
          });
        }
        return colors;
      },
      provideColorPresentations(color) {
        const toHex = (value) => Math.max(0, Math.min(255, Math.round(value * 255)))
          .toString(16).padStart(2, '0').toUpperCase();
        const digits = toHex(color.red) + toHex(color.green) + toHex(color.blue);
        return [new vscode.ColorPresentation('0x' + digits + 'u32')];
      },
    },
  );
  context.subscriptions.push(provider);
}

function registerOfflineCompletions(context) {
  const provider = vscode.languages.registerCompletionItemProvider(
    { scheme: 'file', language: 'jadren' },
    {
      provideCompletionItems(document, position) {
        const source = document.getText();
        const offset = document.offsetAt(position);
        const dot = dotContext(source, offset);
        if (dot) {
          const model = buildLocalModel(source, offset);
          const memberItems = membersForDot(model, dot)
            .filter((member) => !dot.memberPrefix || member.name.toLowerCase().startsWith(dot.memberPrefix.toLowerCase()))
            .map((member) => {
              const kind = member.kind === 'enum'
                ? vscode.CompletionItemKind.EnumMember
                : vscode.CompletionItemKind.Field;
              const item = new vscode.CompletionItem(member.name, kind);
              item.detail = member.kind === 'enum'
                ? `${member.owner}.${member.name}`
                : `${member.name}: ${member.type}`;
              const slovak = documentationLanguage(document) === 'sk';
              item.documentation = new vscode.MarkdownString(
                member.kind === 'enum'
                  ? (slovak ? `Variant enumerácie **${member.owner}**.` : `Enum variant of **${member.owner}**.`)
                  : (slovak ? `Pole typu **${member.owner}** s typom \`${member.type}\`.` : `Field of **${member.owner}** with type \`${member.type}\`.`),
              );
              item.sortText = `-2_${member.name}`;
              item.range = new vscode.Range(document.positionAt(dot.memberStart), position);
              return item;
            });
          return memberItems;
        }
        const line = document.lineAt(position.line).text.slice(0, position.character);
        const match = line.match(/[A-Za-z_][A-Za-z0-9_]*$/);
        const prefix = match ? match[0].toLowerCase() : '';
        const localItems = localSymbolsForCompletion(document, position)
          .filter((symbol) => !prefix || symbol.name.toLowerCase().startsWith(prefix))
          .map((symbol) => {
            const item = new vscode.CompletionItem(symbol.name, symbol.kind);
            item.detail = symbol.detail;
            item.documentation = new vscode.MarkdownString(symbol.documentation);
            item.sortText = `-1_${symbol.name}`;
            if (match) {
              item.range = new vscode.Range(
                position.line,
                position.character - match[0].length,
                position.line,
                position.character,
              );
            }
            return item;
          });
        const coreItems = OFFLINE_COMPLETIONS
          .filter(([label]) => !prefix || label.toLowerCase().startsWith(prefix))
          .map(([label, kind, documentation]) => {
            const item = new vscode.CompletionItem(label, kind);
            item.detail = 'Jadren offline';
            item.documentation = new vscode.MarkdownString(offlineDocumentation(label, documentation, document));
            if (match) {
              item.range = new vscode.Range(
                position.line,
                position.character - match[0].length,
                position.line,
                position.character,
              );
            }
            return item;
          });
        const uiItems = UI_API
          .filter((api) => !prefix || api.name.toLowerCase().startsWith(prefix))
          .map((api) => {
            const item = new vscode.CompletionItem(api.name, vscode.CompletionItemKind.Function);
            item.detail = uiApiSignature(api);
            item.documentation = uiApiMarkdown(api, document);
            item.insertText = new vscode.SnippetString(uiApiSnippet(api));
            item.sortText = `0_${api.name}`;
            if (match) {
              item.range = new vscode.Range(
                position.line,
                position.character - match[0].length,
                position.line,
                position.character,
              );
            }
            return item;
          });
        return [...localItems, ...uiItems, ...coreItems];
      },
    },
    '.',
  );
  context.subscriptions.push(provider);
}

function localFunctionMarkdown(functionEntry, document) {
  const slovak = documentationLanguage(document) === 'sk';
  const parameters = functionEntry.parameters.length === 0
    ? (slovak ? 'Bez parametrov.' : 'No parameters.')
    : functionEntry.parameters
      .map((parameter) => `- \`${parameter.name}\`${parameter.type ? `: \`${parameter.type}\`` : ''}`)
      .join('\n');
  const localFunction = slovak ? 'Lokálna funkcia deklarovaná v tomto súbore Jadren.' : 'Local function declared in this Jadren file.';
  const parametersLabel = slovak ? 'Parametre' : 'Parameters';
  const returnsLabel = slovak ? 'Vracia' : 'Returns';
  return new vscode.MarkdownString(
    `**${functionEntry.name}**\n\n${localFunction}\n\n\`\`\`jadren\n${functionEntry.signature}\n\`\`\`\n\n${parametersLabel}:\n${parameters}\n\n${returnsLabel}: \`${functionEntry.returnType}\``,
  );
}

function localFunctionAtWord(model, word) {
  return model.functions.find((functionEntry) => functionEntry.name === word);
}

function registerOfflineHover(context) {
  const provider = vscode.languages.registerHoverProvider(
    { scheme: 'file', language: 'jadren' },
    {
      provideHover(document, position) {
        const range = document.getWordRangeAtPosition(position, /[A-Za-z_][A-Za-z0-9_]*/);
        if (!range) {
          return undefined;
        }
        const word = document.getText(range);
        const uiApiEntry = UI_API_BY_NAME.get(word);
        if (uiApiEntry) {
          return new vscode.Hover(uiApiMarkdown(uiApiEntry, document), range);
        }
        const model = buildLocalModel(document.getText(), document.offsetAt(position));
        const localFunction = localFunctionAtWord(model, word);
        if (localFunction) {
          return new vscode.Hover(localFunctionMarkdown(localFunction, document), range);
        }
        const localBinding = model.bindings.get(word);
        if (localBinding) {
          const slovak = documentationLanguage(document) === 'sk';
          const type = localBinding.type ? `\n\n${slovak ? 'Typ' : 'Type'}: \`${localBinding.type}\`` : '';
          return new vscode.Hover(
            new vscode.MarkdownString(`**${word}**\n\n${slovak ? 'Lokálna väzba deklarovaná v tejto funkcii Jadren.' : 'Local binding declared in this Jadren function.'}${type}`),
            range,
          );
        }
        const documentation = OFFLINE_HOVER_DOCS.get(word);
        if (!documentation) {
          return undefined;
        }
        return new vscode.Hover(new vscode.MarkdownString(offlineDocumentation(word, documentation, document)), range);
      },
    },
  );
  context.subscriptions.push(provider);
}

function activeUiCall(document, position) {
  const source = document.getText(new vscode.Range(new vscode.Position(0, 0), position));
  const stack = [];
  let quote = '';
  let escaped = false;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (character === '\\') {
        escaped = true;
      } else if (character === quote) {
        quote = '';
      }
      continue;
    }
    if (character === '"') {
      quote = character;
      continue;
    }
    if (character === '(') {
      const before = source.slice(0, index);
      const functionName = before.match(/([A-Za-z_][A-Za-z0-9_]*)\s*$/)?.[1];
      stack.push({ functionName, activeParameter: 0 });
    } else if (character === ')') {
      stack.pop();
    } else if (character === ',' && stack.length > 0) {
      stack[stack.length - 1].activeParameter += 1;
    }
  }
  const call = stack[stack.length - 1];
  const api = call && UI_API_BY_NAME.get(call.functionName);
  return api ? { api, activeParameter: call.activeParameter } : undefined;
}

function registerOfflineSignatureHelp(context) {
  const provider = vscode.languages.registerSignatureHelpProvider(
    { scheme: 'file', language: 'jadren' },
    {
      provideSignatureHelp(document, position) {
        const call = activeUiCall(document, position);
        if (call) {
          const signature = new vscode.SignatureInformation(
            uiApiSignature(call.api),
            uiApiMarkdown(call.api, document),
          );
          signature.parameters = call.api.parameters.map(
            (parameter) => new vscode.ParameterInformation(parameter.label, uiApiParameterDocumentation(parameter, document)),
          );
          const help = new vscode.SignatureHelp();
          help.signatures = [signature];
          help.activeSignature = 0;
          help.activeParameter = Math.min(call.activeParameter, Math.max(0, call.api.parameters.length - 1));
          return help;
        }
        const offset = document.offsetAt(position);
        const model = buildLocalModel(document.getText(), offset);
        const localCall = localActiveCall(document.getText(), offset);
        if (!localCall) return undefined;
        const functionName = localCall.name.split('.').pop();
        const functionEntry = model.functions.find((candidate) => candidate.name === functionName);
        if (!functionEntry) return undefined;
        const signature = new vscode.SignatureInformation(
          functionEntry.signature,
          localFunctionMarkdown(functionEntry, document),
        );
        const slovak = documentationLanguage(document) === 'sk';
        signature.parameters = functionEntry.parameters.map((parameter) => new vscode.ParameterInformation(
          `${parameter.name}${parameter.type ? `: ${parameter.type}` : ''}`,
          parameter.type ? `${slovak ? 'Typ' : 'Type'}: \`${parameter.type}\`` : (slovak ? 'Parameter funkcie.' : 'Function parameter.'),
        ));
        const help = new vscode.SignatureHelp();
        help.signatures = [signature];
        help.activeSignature = 0;
        help.activeParameter = Math.min(
          localCall.activeParameter,
          Math.max(0, functionEntry.parameters.length - 1),
        );
        return help;
      },
    },
    '(',
    ',',
  );
  context.subscriptions.push(provider);
}

const MAX_UPDATE_CATALOG_BYTES = 1024 * 1024;
const MAX_UPDATE_VSIX_BYTES = 128 * 1024 * 1024;
const DEFAULT_UPDATE_SERVER = 'https://jadren.rhsoft.eu/api/releases';

function compareReleaseVersions(left, right) {
  const parse = (value) => {
    const match = String(value || '').trim().match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/);
    if (!match) return undefined;
    return {
      numbers: [Number(match[1]), Number(match[2]), Number(match[3])],
      pre: match[4] ? match[4].split('.') : [],
    };
  };
  const a = parse(left);
  const b = parse(right);
  if (!a || !b) return 0;
  for (let index = 0; index < a.numbers.length; index += 1) {
    if (a.numbers[index] !== b.numbers[index]) return a.numbers[index] > b.numbers[index] ? 1 : -1;
  }
  if (a.pre.length === 0 && b.pre.length === 0) return 0;
  if (a.pre.length === 0) return 1;
  if (b.pre.length === 0) return -1;
  const length = Math.max(a.pre.length, b.pre.length);
  for (let index = 0; index < length; index += 1) {
    if (a.pre[index] === undefined) return -1;
    if (b.pre[index] === undefined) return 1;
    if (a.pre[index] === b.pre[index]) continue;
    const aNumber = /^\d+$/.test(a.pre[index]);
    const bNumber = /^\d+$/.test(b.pre[index]);
    if (aNumber && bNumber) return Number(a.pre[index]) > Number(b.pre[index]) ? 1 : -1;
    if (aNumber !== bNumber) return aNumber ? -1 : 1;
    return a.pre[index] > b.pre[index] ? 1 : -1;
  }
  return 0;
}

function httpsBuffer(url, maximumBytes) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, {
      headers: { Accept: 'application/json, application/octet-stream' },
    }, (response) => {
      if (!response.statusCode || response.statusCode < 200 || response.statusCode >= 300) {
        response.resume();
        reject(new Error(`Update server returned HTTP ${response.statusCode || 'unknown'}.`));
        return;
      }
      const chunks = [];
      let size = 0;
      response.on('data', (chunk) => {
        size += chunk.length;
        if (size > maximumBytes) {
          request.destroy(new Error('Update payload exceeds the configured size limit.'));
          return;
        }
        chunks.push(chunk);
      });
      response.on('end', () => resolve(Buffer.concat(chunks)));
      response.on('error', reject);
    });
    request.setTimeout(10000, () => request.destroy(new Error('Update server timed out.')));
    request.on('error', reject);
  });
}

function trustedUpdateUrl(rawValue, baseUrl) {
  const url = new URL(rawValue, baseUrl);
  if (url.protocol !== 'https:') {
    throw new Error('Jadren update server must use HTTPS.');
  }
  if (url.hostname !== baseUrl.hostname) {
    throw new Error('Jadren update artifact must remain on the configured HTTPS host.');
  }
  return url;
}

async function releaseCatalogForUpdate(configuration) {
  const server = String(configuration.get('server', DEFAULT_UPDATE_SERVER)).trim();
  const catalogUrl = new URL(server);
  if (catalogUrl.protocol !== 'https:') {
    throw new Error('Jadren update server must use HTTPS.');
  }
  const catalog = JSON.parse((await httpsBuffer(catalogUrl, MAX_UPDATE_CATALOG_BYTES)).toString('utf8'));
  if (catalog.schema !== 'jadren-download-catalog-0.1' || typeof catalog.version !== 'string') {
    throw new Error('Jadren update catalog has an unsupported schema.');
  }
  const artifact = Array.isArray(catalog.artifacts)
    ? catalog.artifacts.find((candidate) => candidate.kind === 'vscode-vsix' && candidate.target === 'vscode')
    : undefined;
  if (!artifact || artifact.version !== catalog.version || !/^[a-f0-9]{64}$/i.test(artifact.sha256)) {
    throw new Error('Jadren update catalog has no valid VS Code artifact.');
  }
  if (!Number.isSafeInteger(artifact.bytes) || artifact.bytes <= 0 || artifact.bytes > MAX_UPDATE_VSIX_BYTES) {
    throw new Error('Jadren VSIX size is outside the safe update limit.');
  }
  return {
    catalog,
    artifact,
    artifactUrl: trustedUpdateUrl(artifact.url, catalogUrl),
    catalogUrl,
  };
}

function currentJadrenRelease(context) {
  return context.extension.packageJSON.jadrenRelease
    || context.extension.packageJSON.version
    || '0.0.0';
}

async function installSelfHostedUpdate(context, release) {
  const temporaryDirectory = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'jadren-vscode-update-'));
  const target = path.join(temporaryDirectory, release.artifact.filename || 'jadren-vscode-update.vsix');
  try {
    const payload = await httpsBuffer(release.artifactUrl, MAX_UPDATE_VSIX_BYTES);
    if (payload.length !== release.artifact.bytes) {
      throw new Error(`VSIX size mismatch: expected ${release.artifact.bytes}, received ${payload.length}.`);
    }
    const digest = crypto.createHash('sha256').update(payload).digest('hex');
    if (digest.toLowerCase() !== release.artifact.sha256.toLowerCase()) {
      throw new Error('VSIX SHA-256 does not match the release catalog.');
    }
    await fs.promises.writeFile(target, payload, { flag: 'wx' });
    await vscode.commands.executeCommand('workbench.extensions.installExtension', vscode.Uri.file(target));
    const action = await vscode.window.showInformationMessage(
      `Jadren extension ${release.catalog.version} is installed. Reload VS Code to activate it.`,
      'Reload Window',
    );
    if (action === 'Reload Window') {
      await vscode.commands.executeCommand('workbench.action.reloadWindow');
    }
  } finally {
    await fs.promises.rm(temporaryDirectory, { recursive: true, force: true }).catch(() => undefined);
  }
}

async function checkForSelfHostedUpdate(context, interactive = false) {
  if (updateInFlight) return;
  const configuration = vscode.workspace.getConfiguration('jadren.updates');
  if (!interactive && configuration.get('enabled', true) !== true) return;
  updateInFlight = true;
  try {
    const release = await releaseCatalogForUpdate(configuration);
    const current = currentJadrenRelease(context);
    const comparison = compareReleaseVersions(release.catalog.version, current);
    if (comparison <= 0) {
      if (interactive) {
        vscode.window.showInformationMessage(`Jadren extension is up to date (${current}).`);
      }
      return;
    }
    const action = await vscode.window.showInformationMessage(
      `Jadren extension update available: ${current} → ${release.catalog.version}.`,
      'Update',
      'Later',
    );
    if (action === 'Update') {
      await installSelfHostedUpdate(context, release);
    }
  } catch (error) {
    if (interactive) {
      const message = error instanceof Error ? error.message : String(error);
      vscode.window.showErrorMessage(`Jadren extension update failed: ${message}`);
    }
  } finally {
    updateInFlight = false;
  }
}

function registerUpdateSupport(context) {
  const command = vscode.commands.registerCommand(
    'jadren.checkForUpdates',
    () => checkForSelfHostedUpdate(context, true),
  );
  context.subscriptions.push(command);
  updateTimer = setTimeout(() => checkForSelfHostedUpdate(context), 5000);
  const configuration = vscode.workspace.getConfiguration('jadren.updates');
  const hours = Math.max(1, Math.min(168, Number(configuration.get('checkIntervalHours', 24)) || 24));
  updateInterval = setInterval(() => checkForSelfHostedUpdate(context), hours * 60 * 60 * 1000);
  context.subscriptions.push({
    dispose() {
      if (updateTimer) clearTimeout(updateTimer);
      if (updateInterval) clearInterval(updateInterval);
      updateTimer = undefined;
      updateInterval = undefined;
    },
  });
}

function isJadrenDocument(document) {
  return document && document.languageId === 'jadren' && document.uri.scheme === 'file';
}

async function jadrenDocumentForCommand(resource) {
  if (vscode.Uri.isUri(resource)) {
    return vscode.workspace.openTextDocument(resource);
  }
  return vscode.window.activeTextEditor?.document;
}

function executablePathFor(document, workspaceFolder, configurationKey, fallbackDirectory) {
  const configuration = vscode.workspace.getConfiguration('jadren', document.uri);
  const configuredDirectory = configuration.get(configurationKey, fallbackDirectory);
  const root = workspaceFolder ? workspaceFolder.uri.fsPath : path.dirname(document.uri.fsPath);
  const directory = path.isAbsolute(configuredDirectory)
    ? configuredDirectory
    : path.join(root, configuredDirectory);
  return path.join(directory, `${path.parse(document.uri.fsPath).name}.exe`);
}

function workspaceFolderForDocument(document) {
  return vscode.workspace.getWorkspaceFolder(document.uri)
    || vscode.workspace.workspaceFolders?.[0];
}

function localJadrenCommand(root) {
  if (!root) {
    return undefined;
  }
  const localNames = process.platform === 'win32'
    ? ['target/debug/jadren.exe', 'target/debug/jadren']
    : ['target/debug/jadren'];
  return localNames
    .map((name) => path.join(root, name))
    .find((candidate) => fs.existsSync(candidate));
}

function findJadrenRoot(startPath) {
  let current = startPath;
  while (current) {
    const command = localJadrenCommand(current);
    if (command) {
      return { command, root: current };
    }
    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return undefined;
}

function jadrenCommandFor(document, workspaceFolder) {
  const configuration = vscode.workspace.getConfiguration('jadren', document?.uri);
  const configured = configuration.get('lspPath', 'jadren');
  if (configured !== 'jadren') {
    return configured;
  }
  const root = typeof workspaceFolder === 'string'
    ? workspaceFolder
    : workspaceFolder?.uri?.fsPath;
  const local = localJadrenCommand(root)
    || findJadrenRoot(document?.uri?.fsPath ? path.dirname(document.uri.fsPath) : undefined)?.command;
  if (local) {
    return local;
  }
  return configured;
}

function sourceFileMapFor(document) {
  const sourceDirectory = path.dirname(document.uri.fsPath);
  const normalizedDirectory = sourceDirectory.replace(/\\/g, '/');
  // LLVM records the source directory in the PDB using the compiler's
  // canonical casing and slash separators. cppvsdbg treats sourceFileMap
  // keys as case-sensitive on some versions, so provide exact/lowercase
  // variants for both Windows and LLVM-normalized path spellings.
  return {
    [sourceDirectory]: sourceDirectory,
    [sourceDirectory.toLowerCase()]: sourceDirectory,
    [normalizedDirectory]: sourceDirectory,
    [normalizedDirectory.toLowerCase()]: sourceDirectory,
  };
}

function registerSourceBreakpoint(context) {
  const command = vscode.commands.registerCommand('jadren.toggleSourceBreakpoint', async (resource) => {
    const document = await jadrenDocumentForCommand(resource);
    if (!isJadrenDocument(document)) {
      return;
    }
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.uri.toString() !== document.uri.toString()) {
      return;
    }
    const line = editor.selection.active.line;
    const existing = vscode.debug.breakpoints.filter((breakpoint) => (
      breakpoint instanceof vscode.SourceBreakpoint
      && breakpoint.location.uri.toString() === document.uri.toString()
      && breakpoint.location.range.start.line === line
    ));
    if (existing.length > 0) {
      vscode.debug.removeBreakpoints(existing);
      return;
    }
    vscode.debug.addBreakpoints([
      new vscode.SourceBreakpoint(
        new vscode.Location(document.uri, new vscode.Position(line, 0)),
      ),
    ]);
  });
  context.subscriptions.push(command);
}

function buildExecutable(command, document, workspaceFolder, output, profile, outputChannel) {
  const cwd = workspaceFolder ? workspaceFolder.uri.fsPath : path.dirname(document.uri.fsPath);
  const args = ['build', document.uri.fsPath, '-o', output, '--profile', profile];
  outputChannel.appendLine(`> ${command} ${args.map((value) => JSON.stringify(value)).join(' ')}`);
  return new Promise((resolve, reject) => {
    const process = spawn(command, args, { cwd, windowsHide: true });
    let stderr = '';
    process.stdout.on('data', (data) => outputChannel.append(data.toString()));
    process.stderr.on('data', (data) => {
      const text = data.toString();
      stderr += text;
      outputChannel.append(text);
    });
    process.on('error', (error) => reject(new Error(`Cannot start Jadren CLI '${command}': ${error.message}`)));
    process.on('close', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`Jadren ${profile} build failed with exit code ${code}.${stderr ? ` ${stderr.trim()}` : ''}`));
      }
    });
  });
}

function registerDebugger(context) {
  debugOutput = vscode.window.createOutputChannel('Jadren Debug');
  context.subscriptions.push(debugOutput);
  const command = vscode.commands.registerCommand('jadren.debugCurrentFile', async (resource) => {
    const document = await jadrenDocumentForCommand(resource);
    if (!isJadrenDocument(document)) {
      vscode.window.showErrorMessage('Open a saved .jdn file before starting Jadren Debugger.');
      return;
    }
    await document.save();
    const cppTools = vscode.extensions.getExtension('ms-vscode.cpptools');
    if (!cppTools) {
      vscode.window.showErrorMessage('Jadren Debugger 0.2 requires the Microsoft C/C++ extension (ms-vscode.cpptools).');
      return;
    }
    const workspaceFolder = workspaceFolderForDocument(document);
    const output = executablePathFor(document, workspaceFolder, 'debug.outputDirectory', 'target/jadren/debug');
    const pdb = output.replace(/\.exe$/i, '.pdb');
    const configuration = vscode.workspace.getConfiguration('jadren', document.uri);
    const cli = jadrenCommandFor(document, workspaceFolder);
    const stopAtEntry = configuration.get('debug.stopAtEntry', false);
    debugOutput.show(true);
    try {
      await buildExecutable(cli, document, workspaceFolder, output, 'debug', debugOutput);
      if (!fs.existsSync(output) || !fs.existsSync(pdb)) {
        throw new Error(`Debug build did not produce both executable and PDB: ${output}`);
      }
      const started = await vscode.debug.startDebugging(workspaceFolder, {
        name: `Jadren Debug: ${path.basename(document.uri.fsPath)}`,
        type: 'cppvsdbg',
        request: 'launch',
        program: output,
        cwd: path.dirname(document.uri.fsPath),
        console: 'integratedTerminal',
        internalConsoleOptions: 'neverOpen',
        sourceFileMap: sourceFileMapFor(document),
        // LLVM's current CodeView writer intentionally omits a source
        // checksum. Let cppvsdbg bind the verified line table by path while
        // the checksum emission is added to the compiler in a later slice.
        requireExactSource: false,
        stopAtEntry,
      });
      if (!started) {
        vscode.window.showErrorMessage('VS Code could not start the Jadren debug session. See Jadren Debug output.');
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      debugOutput.appendLine(message);
      vscode.window.showErrorMessage(`Jadren Debugger: ${message}`);
    }
  });
  context.subscriptions.push(command);
}

function registerReleaseBuild(context) {
  buildOutput = vscode.window.createOutputChannel('Jadren Build');
  context.subscriptions.push(buildOutput);
  const command = vscode.commands.registerCommand('jadren.buildReleaseExe', async (resource) => {
    const document = await jadrenDocumentForCommand(resource);
    if (!isJadrenDocument(document)) {
      vscode.window.showErrorMessage('Open or select a saved .jdn file before building a Jadren release EXE.');
      return;
    }
    await document.save();
    const workspaceFolder = workspaceFolderForDocument(document);
    const output = executablePathFor(document, workspaceFolder, 'build.releaseOutputDirectory', 'target/jadren/release');
    const cli = jadrenCommandFor(document, workspaceFolder);
    buildOutput.show(true);
    try {
      await buildExecutable(cli, document, workspaceFolder, output, 'release', buildOutput);
      if (!fs.existsSync(output)) {
        throw new Error(`Release build did not produce an executable: ${output}`);
      }
      buildOutput.appendLine(`Release EXE ready: ${output}`);
      vscode.window.showInformationMessage(`Jadren release EXE ready: ${output}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      buildOutput.appendLine(message);
      vscode.window.showErrorMessage(`Jadren release build: ${message}`);
    }
  });
  context.subscriptions.push(command);
}

function activate(context) {
  registerOfflineCompletions(context);
  registerOfflineHover(context);
  registerOfflineSignatureHelp(context);
  registerColorProvider(context);
  registerDebugger(context);
  registerSourceBreakpoint(context);
  registerReleaseBuild(context);
  registerUpdateSupport(context);
  lspOutput = vscode.window.createOutputChannel('Jadren LSP');
  context.subscriptions.push(lspOutput);
  const startLanguageClient = (document) => {
    if (client) {
      return;
    }
    const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    const command = jadrenCommandFor(document, workspaceFolder);
    const serverOptions = {
      run: {
        command,
        args: ['lsp'],
        transport: TransportKind.stdio,
        options: { cwd: workspaceFolder },
      },
      debug: {
        command,
        args: ['lsp'],
        transport: TransportKind.stdio,
        options: { cwd: workspaceFolder },
      },
    };
    const clientOptions = {
      documentSelector: [{ scheme: 'file', language: 'jadren' }],
      synchronize: {
        configurationSection: 'jadren',
      },
      outputChannel: lspOutput,
      connectionOptions: {
        // A crashed compiler must stay stopped until the user fixes the configured
        // CLI path; automatic restart loops otherwise surface as repeated EPIPEs.
        maxRestartCount: 0,
      },
      errorHandler: {
        error(error, message, count) {
          const detail = error instanceof Error ? error.message : String(error);
          lspOutput.appendLine(`Jadren LSP transport error (${count}): ${message}: ${detail}`);
          return {
            action: ErrorAction.Shutdown,
            handled: true,
          };
        },
        closed() {
          lspOutput.appendLine('Jadren LSP stopped; automatic restart is disabled.');
          return {
            action: CloseAction.DoNotRestart,
            handled: true,
          };
        },
      },
    };
    lspOutput.appendLine(`Jadren LSP command: ${command}`);
    client = new LanguageClient(
      'jadrenLanguageServer',
      'Jadren Language Server',
      serverOptions,
      clientOptions,
    );
    context.subscriptions.push(client.start());
  };
  const startupDocument = vscode.window.activeTextEditor?.document
    || vscode.workspace.textDocuments.find((document) => isJadrenDocument(document));
  if (startupDocument || vscode.workspace.workspaceFolders?.length) {
    startLanguageClient(startupDocument);
  } else {
    const startOnDocument = (document) => {
      if (isJadrenDocument(document)) {
        startLanguageClient(document);
      }
    };
    context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(startOnDocument));
    context.subscriptions.push(vscode.window.onDidChangeActiveTextEditor((editor) => {
      startOnDocument(editor?.document);
    }));
  }
}

function deactivate() {
  if (updateTimer) clearTimeout(updateTimer);
  if (updateInterval) clearInterval(updateInterval);
  updateTimer = undefined;
  updateInterval = undefined;
  return client?.stop();
}

module.exports = {
  activate,
  deactivate,
};
