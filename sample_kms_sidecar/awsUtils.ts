import {
    KMSClient,
    SignCommand,
    VerifyCommand,
    GetPublicKeyCommand,
} from "@aws-sdk/client-kms";

import { Secp256k1PublicKey } from "@mysten/sui.js/keypairs/secp256k1";
import { fromB64, toB64 } from "@mysten/sui.js/utils";

import {
    toSerializedSignature,
    SIGNATURE_FLAG_TO_SCHEME,
    SignatureScheme,
    SignatureFlag,
    SerializedSignature,
    messageWithIntent,
    IntentScope,
} from "@mysten/sui.js/cryptography";

import { TransactionBlock } from "@mysten/sui.js/transactions";
import { getFullnodeUrl, SuiClient } from "@mysten/sui.js/client";
import { blake2b } from "@noble/hashes/blake2b";

import { secp256k1 } from "@noble/curves/secp256k1";

import * as asn1ts from "asn1-ts";

// https://datatracker.ietf.org/doc/html/rfc5480#section-2.2
// https://www.secg.org/sec1-v2.pdf
function bitsToBytes(bitsArray: Uint8ClampedArray) {
    const bytes = new Uint8Array(65);
    for (let i = 0; i < 520; i++) {
        if (bitsArray[i] === 1) {
            bytes[Math.floor(i / 8)] |= 1 << (7 - (i % 8));
        }
    }
    return bytes;
}

function compressPublicKeyClamped(
    uncompressedKey: Uint8ClampedArray,
): Uint8Array {
    if (uncompressedKey.length !== 520) {
        throw new Error("Unexpected length for an uncompressed public key");
    }

    // Convert bits to bytes
    const uncompressedBytes = bitsToBytes(uncompressedKey);
    //console.log("Uncompressed Bytes:", uncompressedBytes);

    // Check if the first byte is 0x04
    if (uncompressedBytes[0] !== 0x04) {
        throw new Error("Public key does not start with 0x04");
    }

    // Extract X-Coordinate (skip the first byte, which should be 0x04)
    const xCoord = uncompressedBytes.slice(1, 33);

    // Determine parity byte for y coordinate
    const yCoordLastByte = uncompressedBytes[64];
    const parityByte = yCoordLastByte % 2 === 0 ? 0x02 : 0x03;

    return new Uint8Array([parityByte, ...xCoord]);
}

function createAWSKMSClient() {
    const client = new KMSClient({
        region: process.env.AWS_REGION || "",
        credentials: {
            accessKeyId: process.env.AWS_ACCESS_KEY_ID || "",
            secretAccessKey: process.env.AWS_SECRET_ACCESS_KEY || "",
        },
    });
    return client;
}

export async function getPublicKey(keyId: string) {
    // gets AWS KMS Public Key in DER format
    // returns Sui Public Key

    // AWS KMS client configuration
    const client = createAWSKMSClient();

    try {
        const getPublicKeyCommand = new GetPublicKeyCommand({
            KeyId: keyId,
        });
        const publicKeyResponse = await client.send(getPublicKeyCommand);
        const publicKey = publicKeyResponse.PublicKey || new Uint8Array();

        let encodedData: Uint8Array = publicKey;
        let derElement: asn1ts.DERElement = new asn1ts.DERElement();
        derElement.fromBytes(encodedData);

        // https://datatracker.ietf.org/doc/html/rfc5480#section-2.2
        // parse ANS1 DER response and get raw public key bytes
        // actual result is a BIT_STRING of length 520-bits (65 bytes)
        if (
            derElement.tagClass === asn1ts.ASN1TagClass.universal &&
            derElement.construction === asn1ts.ASN1Construction.constructed
        ) {
            let components = derElement.components;
            let publicKeyElement = components[1];

            let rawPublicKey = publicKeyElement.bitString; // bitString creates a Uint8ClampedArray;

            const compressedKey = compressPublicKeyClamped(rawPublicKey);

            const sui_public_key = rawPublicKey
                ? new Secp256k1PublicKey(compressedKey)
                : "";
            if (sui_public_key instanceof Secp256k1PublicKey) {
                console.log("Sui Public Key:", sui_public_key.toSuiAddress());
            }
            return sui_public_key;
        } else {
            throw new Error("Unexpected ASN.1 structure");
        }
    } catch (error) {
        console.error("Error during get public key:", error);
        return;
    }
}

function getConcatenatedSignature(signature: Uint8Array): Uint8Array {
    // creates signature consumable by Sui 'toSerializedSignature' call

    // start processing signature
    // populate concatenatedSignature with [r,s] from DER signature

    let encodedData: Uint8Array = signature || new Uint8Array();
    let derElement: asn1ts.DERElement = new asn1ts.DERElement();
    derElement.fromBytes(encodedData);
    let der_json_data: { value: string }[] = derElement.toJSON() as {
        value: any;
    }[];

    const new_r = der_json_data[0];
    const new_s = der_json_data[1];
    //console.log(String(new_r));

    const new_r_string = String(new_r);
    const new_s_string = String(new_s);

    const secp256k1_sig = new secp256k1.Signature(
        BigInt(new_r_string),
        BigInt(new_s_string),
    );
    const secp256k1_normalize_s_compact = secp256k1_sig
        .normalizeS()
        .toCompactRawBytes();

    return secp256k1_normalize_s_compact;
}

async function getSerializedSignature(
    signature: Uint8Array,
    sui_pubkey: Secp256k1PublicKey,
) {
    // create serialized signature from [r,s] and public key
    // Sui Serialized Signature format: `flag || sig || pk`.
    const flag = sui_pubkey ? sui_pubkey.flag() : 1;

    // Check if flag is one of the allowed values and cast to SignatureFlag
    const allowedFlags: SignatureFlag[] = [0, 1, 2, 3, 5];
    const isAllowedFlag = allowedFlags.includes(flag as SignatureFlag);

    const signature_scheme: SignatureScheme = isAllowedFlag
        ? SIGNATURE_FLAG_TO_SCHEME[flag as SignatureFlag]
        : "Secp256k1";

    const publicKeyToUse =
        sui_pubkey instanceof Secp256k1PublicKey ? sui_pubkey : undefined;

    // Call toSerializedSignature
    const serializedSignature: SerializedSignature = toSerializedSignature({
        signatureScheme: signature_scheme,
        signature: signature,
        publicKey: publicKeyToUse,
    });
    return serializedSignature;
}

export async function signAndVerify(tx_bytes: Uint8Array) {
    // sign sui transaction using AWS KMS
    // verify sui transaction using sui public key
    // verify signature using AWS KMS

    const keyId = process.env.AWS_KMS_KEY_ID || "";
    const client = createAWSKMSClient();

    // add intent Message to Transaction Bytes
    const intentMessage = messageWithIntent(
        IntentScope.TransactionData,
        tx_bytes,
    );
    // digest needs to be hash of intent message
    // H(intent | tx_bytes)
    const digest = blake2b(intentMessage, { dkLen: 32 });
    console.log("TX Bytes:", toB64(tx_bytes));
    console.log("Digest:", toB64(digest));

    try {
        // Sign the digest
        const signCommand = new SignCommand({
            KeyId: keyId,
            Message: digest,
            MessageType: "RAW",
            SigningAlgorithm: "ECDSA_SHA_256", // Adjust the algorithm based on your key spec
        });
        const signResponse = await client.send(signCommand);
        const signature = signResponse.Signature || new Uint8Array();

        const original_pubkey = await getPublicKey(keyId);
        const publicKeyToUse =
            original_pubkey instanceof Secp256k1PublicKey
                ? original_pubkey
                : undefined;

        const concatenatedSignature = getConcatenatedSignature(signature);
        const serializedSignature = await getSerializedSignature(
            concatenatedSignature,
            publicKeyToUse as Secp256k1PublicKey,
        );

        console.log("Serialized Signature:", serializedSignature);

        // verify signature with sui
        if (publicKeyToUse !== undefined) {
            console.log("Verifying Sui Signature against TX");
            const isValid = await publicKeyToUse.verifyTransactionBlock(
                tx_bytes,
                serializedSignature,
            );
            console.log("Sui Signature valid:", isValid);
        }

        // Verify the signature in KMS
        const verifyCommand = new VerifyCommand({
            KeyId: keyId,
            Message: digest,
            MessageType: "RAW",
            Signature: signature,
            SigningAlgorithm: "ECDSA_SHA_256",
        });
        const verifyResponse = await client.send(verifyCommand);
        console.log("KMS Signature valid:", verifyResponse.SignatureValid);

        return serializedSignature;
    } catch (error) {
        console.error("Error during sign/verify:", error);
    }
}

