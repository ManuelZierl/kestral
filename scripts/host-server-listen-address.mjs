const LISTEN_MESSAGE = /Kestral backend listening on (http:\/\/[^\s]+)\r?\n/;

export function parseHostServerListenAddress(output) {
  return output.match(LISTEN_MESSAGE)?.[1];
}
