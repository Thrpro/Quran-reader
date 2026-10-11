export type Recording = { id: number; name: string; server: string; surah_list: string };
export type Reciter = { id: number; name: string; moshaf: Recording[] };
export type AyahReciter = {
  id: number;
  nameEnglish: string;
  nameArabic: string;
  styleEnglish?: string;
  styleArabic?: string;
  path: string;
  provider?: "everyayah";
};

// Quran Foundation's public ayah-by-ayah recitation catalog. These IDs are
// intentionally separate from MP3Quran's whole-surah reciter IDs.
export const AYAH_RECITERS: readonly AyahReciter[] = [
  { id: 1001, nameEnglish: "Yasser Al-Dosari", nameArabic: "ياسر الدوسري", path: "Yasser_Ad-Dussary_128kbps", provider: "everyayah" },
  { id: 1002, nameEnglish: "Maher Al-Muaiqly", nameArabic: "ماهر المعيقلي", path: "MaherAlMuaiqly128kbps", provider: "everyayah" },
  { id: 1003, nameEnglish: "Nasser Al-Qatami", nameArabic: "ناصر القطامي", path: "Nasser_Alqatami_128kbps", provider: "everyayah" },
  { id: 1004, nameEnglish: "Fares Abbad", nameArabic: "فارس عباد", path: "Fares_Abbad_64kbps", provider: "everyayah" },
  { id: 1005, nameEnglish: "Abdullah Al-Juhany", nameArabic: "عبد الله الجهني", path: "Abdullaah_3awwaad_Al-Juhaynee_128kbps", provider: "everyayah" },
  { id: 2, nameEnglish: "AbdulBaset AbdulSamad", nameArabic: "عبد الباسط عبد الصمد", styleEnglish: "Murattal", styleArabic: "مرتل", path: "AbdulBaset/Murattal" },
  { id: 1, nameEnglish: "AbdulBaset AbdulSamad", nameArabic: "عبد الباسط عبد الصمد", styleEnglish: "Mujawwad", styleArabic: "مجود", path: "AbdulBaset/Mujawwad" },
  { id: 3, nameEnglish: "Abdur-Rahman as-Sudais", nameArabic: "عبد الرحمن السديس", path: "Sudais" },
  { id: 4, nameEnglish: "Abu Bakr al-Shatri", nameArabic: "أبو بكر الشاطري", path: "Shatri" },
  { id: 5, nameEnglish: "Hani ar-Rifai", nameArabic: "هاني الرفاعي", path: "Rifai" },
  { id: 12, nameEnglish: "Mahmoud Khalil Al-Husary", nameArabic: "محمود خليل الحصري", styleEnglish: "Muallim", styleArabic: "معلم", path: "Husary/Muallim" },
  { id: 6, nameEnglish: "Mahmoud Khalil Al-Husary", nameArabic: "محمود خليل الحصري", path: "Husary" },
  { id: 7, nameEnglish: "Mishari Rashid Alafasy", nameArabic: "مشاري راشد العفاسي", path: "Alafasy" },
  { id: 9, nameEnglish: "Mohamed Siddiq al-Minshawi", nameArabic: "محمد صديق المنشاوي", styleEnglish: "Murattal", styleArabic: "مرتل", path: "Minshawy/Murattal" },
  { id: 8, nameEnglish: "Mohamed Siddiq al-Minshawi", nameArabic: "محمد صديق المنشاوي", styleEnglish: "Mujawwad", styleArabic: "مجود", path: "Minshawy/Mujawwad" },
  { id: 10, nameEnglish: "Saud ash-Shuraym", nameArabic: "سعود الشريم", path: "Shuraym" },
  { id: 11, nameEnglish: "Mohamed al-Tablawi", nameArabic: "محمد الطبلاوي", path: "Tablawi" },
];

export function ayahAudioUrl(reciter: AyahReciter, surah: number, ayah: number) {
  if (!Number.isInteger(surah) || surah < 1 || surah > 114 || !Number.isInteger(ayah) || ayah < 1) return "";
  const file = `${String(surah).padStart(3, "0")}${String(ayah).padStart(3, "0")}.mp3`;
  if (reciter.provider === "everyayah") return `https://everyayah.com/data/${reciter.path}/${file}`;
  return `https://verses.quran.foundation/${reciter.path}/mp3/${file}`;
}

export function reciterDisplayName(reciter: AyahReciter, language: "arabic" | "english") {
  const name = language === "arabic" ? reciter.nameArabic : reciter.nameEnglish;
  const style = language === "arabic" ? reciter.styleArabic : reciter.styleEnglish;
  return style ? `${name} (${style})` : name;
}

export function normalizeReciterSearch(value: string) {
  return value.normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[أإآٱ]/g, "ا")
    .replace(/ى/g, "ي")
    .replace(/ؤ/g, "و")
    .replace(/ئ/g, "ي")
    .replace(/ة/g, "ه")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

function editDistanceWithin(left: string, right: string, limit: number) {
  if (Math.abs(left.length - right.length) > limit) return false;
  let previous = Array.from({ length: right.length + 1 }, (_, index) => index);
  for (let row = 1; row <= left.length; row += 1) {
    const current = [row];
    let rowMinimum = row;
    for (let column = 1; column <= right.length; column += 1) {
      const value = Math.min(
        current[column - 1] + 1,
        previous[column] + 1,
        previous[column - 1] + (left[row - 1] === right[column - 1] ? 0 : 1),
      );
      current.push(value);
      rowMinimum = Math.min(rowMinimum, value);
    }
    if (rowMinimum > limit) return false;
    previous = current;
  }
  return previous[right.length] <= limit;
}

export function reciterNameMatches(name: string, query: string) {
  const normalizedName = normalizeReciterSearch(name);
  const normalizedQuery = normalizeReciterSearch(query);
  if (!normalizedQuery || normalizedName.includes(normalizedQuery)) return true;
  const nameTokens = normalizedName.split(" ").filter(Boolean);
  return normalizedQuery.split(" ").filter(Boolean).every(queryToken =>
    nameTokens.some(nameToken => {
      if (nameToken.includes(queryToken) || nameToken.length >= 4 && queryToken.includes(nameToken)) return true;
      const longest = Math.max(nameToken.length, queryToken.length);
      const tolerance = longest >= 8 ? 2 : longest >= 4 ? 1 : 0;
      return tolerance > 0 && editDistanceWithin(nameToken, queryToken, tolerance);
    }),
  );
}

export function readReciterCatalog(value: unknown): Reciter[] {
  const source = Array.isArray(value)
    ? value
    : value && typeof value === "object" && Array.isArray((value as { reciters?: unknown }).reciters)
      ? (value as { reciters: unknown[] }).reciters
      : [];

  return source.flatMap((candidate): Reciter[] => {
    if (!candidate || typeof candidate !== "object") return [];
    const value = candidate as { id?: unknown; name?: unknown; moshaf?: unknown };
    if (!Number.isSafeInteger(value.id) || typeof value.name !== "string" || !Array.isArray(value.moshaf)) return [];

    const moshaf = value.moshaf.flatMap((candidateRecording): Recording[] => {
      if (!candidateRecording || typeof candidateRecording !== "object") return [];
      const recording = candidateRecording as {
        id?: unknown; name?: unknown; server?: unknown; surah_list?: unknown;
      };
      if (!Number.isSafeInteger(recording.id) || typeof recording.name !== "string" ||
          typeof recording.server !== "string" || typeof recording.surah_list !== "string") return [];
      return [{
        id: recording.id as number,
        name: recording.name,
        server: recording.server,
        surah_list: recording.surah_list,
      }];
    });
    return moshaf.length ? [{ id: value.id as number, name: value.name, moshaf }] : [];
  });
}

export function audioUrl(recording: Recording | undefined, surah: number) {
  if (!recording || !recording.surah_list.split(",").includes(String(surah))) return "";
  try {
    const server = new URL(recording.server);
    if (server.protocol !== "https:" || !server.hostname.endsWith(".mp3quran.net")) return "";
    return `${server.href.replace(/\/$/, "")}/${String(surah).padStart(3, "0")}.mp3`;
  } catch {
    return "";
  }
}
