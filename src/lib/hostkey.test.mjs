// Run with "npm test" (Node 24 reads the .ts file directly).
import assert from "node:assert/strict";
import { test } from "node:test";
import { identityCommand, identityText } from "./hostkey.ts";

test("the command to check an identity names the key file of that kind", () => {
  assert.equal(identityCommand("ssh-ed25519", "linux"), "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub");
  assert.equal(identityCommand("ecdsa-sha2-nistp256", "proxmox"), "ssh-keygen -lf /etc/ssh/ssh_host_ecdsa_key.pub");
  assert.equal(identityCommand("ssh-rsa", "other"), "ssh-keygen -lf /etc/ssh/ssh_host_rsa_key.pub");
  assert.equal(identityCommand("rsa-sha2-512", "linux"), "ssh-keygen -lf /etc/ssh/ssh_host_rsa_key.pub");
});

test("on Windows the key files are in ProgramData", () => {
  assert.equal(identityCommand("ssh-ed25519", "windows"), "ssh-keygen -lf C:\\ProgramData\\ssh\\ssh_host_ed25519_key.pub");
  assert.equal(identityCommand(" SSH-RSA ", "windows"), "ssh-keygen -lf C:\\ProgramData\\ssh\\ssh_host_rsa_key.pub");
});

test("a kind home-cc can't name still gives a command to start from", () => {
  assert.equal(identityCommand("", "linux"), "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub");
  assert.equal(identityCommand("something-new", "linux"), "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub");
});

test("an identity is written as its kind, then its fingerprint", () => {
  assert.equal(identityText({ algorithm: "ssh-ed25519", fingerprint: "SHA256:abc" }), "ssh-ed25519 SHA256:abc");
  assert.equal(identityText({ algorithm: "", fingerprint: " SHA256:abc " }), "SHA256:abc");
});
