type JsonCredentialDescriptor = Omit<PublicKeyCredentialDescriptor, "id"> & { id: string };
type JsonCreationOptions = Omit<
  PublicKeyCredentialCreationOptions,
  "challenge" | "user" | "excludeCredentials"
> & {
  challenge: string;
  user: Omit<PublicKeyCredentialUserEntity, "id"> & { id: string };
  excludeCredentials?: JsonCredentialDescriptor[];
};
type JsonRequestOptions = Omit<
  PublicKeyCredentialRequestOptions,
  "challenge" | "allowCredentials"
> & {
  challenge: string;
  allowCredentials?: JsonCredentialDescriptor[];
};

interface CreationChallenge {
  publicKey: JsonCreationOptions;
}

interface RequestChallenge {
  publicKey: JsonRequestOptions;
}

export function passkeysAvailable(): boolean {
  return typeof PublicKeyCredential !== "undefined" && Boolean(navigator.credentials);
}

export function decodeBase64Url(value: string): ArrayBuffer {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes.buffer;
}

export function encodeBase64Url(value: ArrayBuffer): string {
  const bytes = new Uint8Array(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function creationOptionsFromJson(challenge: CreationChallenge): CredentialCreationOptions {
  const options = challenge.publicKey;
  return {
    publicKey: {
      ...options,
      challenge: decodeBase64Url(options.challenge),
      user: { ...options.user, id: decodeBase64Url(options.user.id) },
      excludeCredentials: options.excludeCredentials?.map((credential) => ({
        ...credential,
        id: decodeBase64Url(credential.id),
      })),
    },
  };
}

export function requestOptionsFromJson(challenge: RequestChallenge): CredentialRequestOptions {
  const options = challenge.publicKey;
  return {
    publicKey: {
      ...options,
      challenge: decodeBase64Url(options.challenge),
      allowCredentials: options.allowCredentials?.map((credential) => ({
        ...credential,
        id: decodeBase64Url(credential.id),
      })),
    },
  };
}

export function registrationCredentialToJson(credential: PublicKeyCredential): unknown {
  const response = credential.response as AuthenticatorAttestationResponse;
  const transports = typeof response.getTransports === "function" ? response.getTransports() : undefined;
  return {
    id: credential.id,
    rawId: encodeBase64Url(credential.rawId),
    response: {
      attestationObject: encodeBase64Url(response.attestationObject),
      clientDataJSON: encodeBase64Url(response.clientDataJSON),
      ...(transports ? { transports } : {}),
    },
    type: credential.type,
    extensions: credential.getClientExtensionResults(),
  };
}

export function authenticationCredentialToJson(credential: PublicKeyCredential): unknown {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: encodeBase64Url(credential.rawId),
    response: {
      authenticatorData: encodeBase64Url(response.authenticatorData),
      clientDataJSON: encodeBase64Url(response.clientDataJSON),
      signature: encodeBase64Url(response.signature),
      userHandle: response.userHandle ? encodeBase64Url(response.userHandle) : null,
    },
    type: credential.type,
    extensions: credential.getClientExtensionResults(),
  };
}

export async function createPasskey(challenge: unknown): Promise<unknown> {
  requirePasskeys();
  const credential = await navigator.credentials.create(
    creationOptionsFromJson(challenge as CreationChallenge),
  );
  if (!(credential instanceof PublicKeyCredential)) throw new Error("Passkey creation returned no credential");
  return registrationCredentialToJson(credential);
}

export async function getPasskey(challenge: unknown): Promise<unknown> {
  requirePasskeys();
  const credential = await navigator.credentials.get(
    requestOptionsFromJson(challenge as RequestChallenge),
  );
  if (!(credential instanceof PublicKeyCredential)) throw new Error("Passkey sign-in returned no credential");
  return authenticationCredentialToJson(credential);
}

function requirePasskeys(): void {
  if (!passkeysAvailable()) {
    throw new Error("This browser does not support passkeys. Use a current browser over HTTPS.");
  }
}
