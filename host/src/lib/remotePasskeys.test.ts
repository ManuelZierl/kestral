import { describe, expect, it } from "vitest";
import {
  creationOptionsFromJson,
  decodeBase64Url,
  encodeBase64Url,
  requestOptionsFromJson,
} from "$lib/remotePasskeys";

describe("remote passkey wire conversion", () => {
  function bytes(value: BufferSource): number[] {
    const view = value instanceof ArrayBuffer
      ? new Uint8Array(value)
      : new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    return Array.from(view);
  }

  it("round trips URL-safe unpadded base64", () => {
    const bytes = new Uint8Array([0, 1, 2, 250, 251, 252]).buffer;
    const encoded = encodeBase64Url(bytes);

    expect(encoded).toBe("AAEC-vv8");
    expect(Array.from(new Uint8Array(decodeBase64Url(encoded)))).toEqual([0, 1, 2, 250, 251, 252]);
  });

  it("converts registration challenge, owner id, and excluded credential ids", () => {
    const options = creationOptionsFromJson({
      publicKey: {
        challenge: "AQI",
        rp: { id: "kestral.example", name: "Kestral" },
        user: { id: "AwQ", name: "owner", displayName: "Owner" },
        pubKeyCredParams: [{ type: "public-key", alg: -7 }],
        excludeCredentials: [{ type: "public-key", id: "BQY" }],
      },
    });

    expect(bytes(options.publicKey!.challenge)).toEqual([1, 2]);
    expect(bytes(options.publicKey!.user.id)).toEqual([3, 4]);
    expect(bytes(options.publicKey!.excludeCredentials![0].id)).toEqual([5, 6]);
  });

  it("converts authentication challenge and allowed credential ids", () => {
    const options = requestOptionsFromJson({
      publicKey: {
        challenge: "Bwg",
        rpId: "kestral.example",
        allowCredentials: [{ type: "public-key", id: "CQo" }],
      },
    });

    expect(bytes(options.publicKey!.challenge)).toEqual([7, 8]);
    expect(bytes(options.publicKey!.allowCredentials![0].id)).toEqual([9, 10]);
  });
});
