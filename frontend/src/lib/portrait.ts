import { PersonaGender } from "~/gen/famulen/v1/settings_pb";

/** The two secretaries in the brand, and what tells them apart. */
export const PORTRAITS = [
  { id: "male", gender: PersonaGender.MALE, label: "男性", note: "口ひげに丸眼鏡" },
  { id: "female", gender: PersonaGender.FEMALE, label: "女性", note: "まとめ髪に丸眼鏡" },
] as const;

export type Portrait = (typeof PORTRAITS)[number]["id"];

/** The portrait of a gender; the first one until it is known. */
export function portraitOf(gender: PersonaGender | undefined): Portrait {
  return (PORTRAITS.find((p) => p.gender === gender) ?? PORTRAITS[0]).id;
}

/** Where the console serves one of a portrait's files. */
export const iconFile = (portrait: Portrait, file: string) => `/icons/${portrait}/${file}`;
