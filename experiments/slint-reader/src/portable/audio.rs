//! Linux and macOS: the native reciter list, word for word (a Windows test
//! compares it with audio.rs). Playback itself is in portable/audio_bridge.rs.

#[derive(Debug, PartialEq, Eq)]
pub struct Reciter {
    pub english: &'static str,
    pub arabic: &'static str,
    pub path: &'static str,
    pub everyayah: bool,
}
pub const RECITERS: &[Reciter] = &[
    Reciter {
        english: "Yasser Al-Dosari",
        arabic: "ياسر الدوسري",
        path: "Yasser_Ad-Dussary_128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Maher Al-Muaiqly",
        arabic: "ماهر المعيقلي",
        path: "MaherAlMuaiqly128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Nasser Al-Qatami",
        arabic: "ناصر القطامي",
        path: "Nasser_Alqatami_128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Fares Abbad",
        arabic: "فارس عباد",
        path: "Fares_Abbad_64kbps",
        everyayah: true,
    },
    Reciter {
        english: "Abdullah Al-Juhany",
        arabic: "عبد الله الجهني",
        path: "Abdullaah_3awwaad_Al-Juhaynee_128kbps",
        everyayah: true,
    },
    Reciter {
        english: "Mishari Rashid Alafasy",
        arabic: "مشاري راشد العفاسي",
        path: "Alafasy",
        everyayah: false,
    },
    Reciter {
        english: "AbdulBaset AbdulSamad (Murattal)",
        arabic: "عبد الباسط عبد الصمد (مرتل)",
        path: "AbdulBaset/Murattal",
        everyayah: false,
    },
    Reciter {
        english: "AbdulBaset AbdulSamad (Mujawwad)",
        arabic: "عبد الباسط عبد الصمد (مجود)",
        path: "AbdulBaset/Mujawwad",
        everyayah: false,
    },
    Reciter {
        english: "Abdur-Rahman as-Sudais",
        arabic: "عبد الرحمن السديس",
        path: "Sudais",
        everyayah: false,
    },
    Reciter {
        english: "Abu Bakr al-Shatri",
        arabic: "أبو بكر الشاطري",
        path: "Shatri",
        everyayah: false,
    },
    Reciter {
        english: "Hani ar-Rifai",
        arabic: "هاني الرفاعي",
        path: "Rifai",
        everyayah: false,
    },
    Reciter {
        english: "Mahmoud Khalil Al-Husary (Muallim)",
        arabic: "محمود خليل الحصري (معلم)",
        path: "Husary/Muallim",
        everyayah: false,
    },
    Reciter {
        english: "Mahmoud Khalil Al-Husary",
        arabic: "محمود خليل الحصري",
        path: "Husary",
        everyayah: false,
    },
    Reciter {
        english: "Mohamed Siddiq al-Minshawi (Murattal)",
        arabic: "محمد صديق المنشاوي (مرتل)",
        path: "Minshawy/Murattal",
        everyayah: false,
    },
    Reciter {
        english: "Mohamed Siddiq al-Minshawi (Mujawwad)",
        arabic: "محمد صديق المنشاوي (مجود)",
        path: "Minshawy/Mujawwad",
        everyayah: false,
    },
    Reciter {
        english: "Saud ash-Shuraym",
        arabic: "سعود الشريم",
        path: "Shuraym",
        everyayah: false,
    },
    Reciter {
        english: "Mohamed al-Tablawi",
        arabic: "محمد الطبلاوي",
        path: "Tablawi",
        everyayah: false,
    },
];
