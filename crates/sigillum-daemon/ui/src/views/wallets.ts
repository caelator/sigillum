export type WalletProfileKind = "stealth" | "xpub" | "seed";

export interface WalletProfileView {
  name: string;
  kind: WalletProfileKind;
  provider_profile?: string | null;
  signer_available?: boolean | null;
}

export interface WalletFamilyCounts {
  stealth: number;
  xpub: number;
  seed: number;
}

export function countWalletFamilies(profiles: WalletProfileView[]): WalletFamilyCounts {
  return profiles.reduce<WalletFamilyCounts>(
    (counts, profile) => ({
      ...counts,
      [profile.kind]: counts[profile.kind] + 1,
    }),
    { stealth: 0, xpub: 0, seed: 0 },
  );
}

export function hasOperationalWallet(profiles: WalletProfileView[]): boolean {
  return profiles.some(
    (profile) => profile.kind === "stealth" && profile.signer_available !== false,
  );
}
