//! Toneless Mandarin pinyin syllable table.
//!
//! The table holds 425 entries covering every toneless spelling used by the
//! bundled luna_pinyin dictionary, including rare/dialect readings (`biang`,
//! `eh`, `lvan`, ...) and both `lue`/`lve` aliases. Segmentation, dictionary
//! key building and dictionary loading all consult this single source of
//! truth through [`is_syllable`].

/// All toneless Mandarin pinyin syllables (lowercase, `v` spellings for
/// `u:`-umlaut). Roughly grouped by initial consonant.
pub const SYLLABLES: &[&str] = &[
    "a", "ai", "an", "ang", "ao", "ba", "bai", "ban", "bang", "bao", "bei", "ben", "beng", "bi",
    "bian", "biang", "biao", "bie", "bin", "bing", "bo", "bu", "ca", "cai", "can", "cang", "cao",
    "ce", "cei", "cen", "ceng", "cha", "chai", "chan", "chang", "chao", "che", "chen", "cheng",
    "chi", "chong", "chou", "chu", "chua", "chuai", "chuan", "chuang", "chui", "chun", "chuo",
    "ci", "cong", "cou", "cu", "cuan", "cui", "cun", "cuo", "da", "dai", "dan", "dang", "dao",
    "de", "dei", "den", "deng", "di", "dia", "dian", "diao", "die", "din", "ding", "diu", "dong",
    "dou", "du", "duan", "dui", "dun", "duo", "e", "eh", "ei", "en", "eng", "er", "fa", "fan",
    "fang", "fei", "fen", "feng", "fiao", "fo", "fong", "fou", "fu", "ga", "gai", "gan", "gang",
    "gao", "ge", "gei", "gen", "geng", "gong", "gou", "gu", "gua", "guai", "guan", "guang", "gui",
    "gun", "guo", "ha", "hai", "han", "hang", "hao", "he", "hei", "hen", "heng", "hong", "hou",
    "hu", "hua", "huai", "huan", "huang", "hui", "hun", "huo", "ji", "jia", "jian", "jiang",
    "jiao", "jie", "jin", "jing", "jiong", "jiu", "ju", "juan", "jue", "jun", "ka", "kai", "kan",
    "kang", "kao", "ke", "kei", "ken", "keng", "kong", "kou", "ku", "kua", "kuai", "kuan", "kuang",
    "kui", "kun", "kuo", "la", "lai", "lan", "lang", "lao", "le", "lei", "leng", "li", "lia",
    "lian", "liang", "liao", "lie", "lin", "ling", "liu", "lo", "long", "lou", "lu", "luan", "lun",
    "luo", "lv", "lue", "lve", "lvan", "ma", "mai", "man", "mang", "mao", "me", "mei", "men",
    "meng", "mi", "mian", "miao", "mie", "min", "ming", "miu", "mo", "mou", "mu", "na", "nai",
    "nan", "nang", "nao", "ne", "nei", "nen", "neng", "ni", "nia", "nian", "niang", "niao", "nie",
    "nin", "ning", "niu", "nong", "nou", "nu", "nuan", "nun", "nuo", "nv", "nve", "o", "ou", "pa",
    "pai", "pan", "pang", "pao", "pei", "pen", "peng", "pi", "pia", "pian", "piao", "pie", "pin",
    "ping", "po", "pou", "pu", "qi", "qia", "qian", "qiang", "qiao", "qie", "qin", "qing", "qiong",
    "qiu", "qu", "quan", "que", "qun", "ran", "rang", "rao", "re", "ren", "reng", "ri", "rong",
    "rou", "ru", "rua", "ruan", "rui", "run", "ruo", "sa", "sai", "san", "sang", "sao", "se",
    "sei", "sen", "seng", "sha", "shai", "shan", "shang", "shao", "she", "shei", "shen", "sheng",
    "shi", "shou", "shu", "shua", "shuai", "shuan", "shuang", "shui", "shun", "shuo", "si", "song",
    "sou", "su", "suan", "sui", "sun", "suo", "ta", "tai", "tan", "tang", "tao", "te", "tei",
    "teng", "ti", "tian", "tiao", "tie", "ting", "tong", "tou", "tu", "tuan", "tui", "tun", "tuo",
    "wa", "wai", "wan", "wang", "wei", "wen", "weng", "wo", "wong", "wu", "xi", "xia", "xian",
    "xiang", "xiao", "xie", "xin", "xing", "xiong", "xiu", "xu", "xuan", "xue", "xun", "ya", "yai",
    "yan", "yang", "yao", "ye", "yi", "yin", "ying", "yo", "yong", "you", "yu", "yuan", "yun",
    "yue", "za", "zai", "zan", "zang", "zao", "ze", "zei", "zen", "zeng", "zha", "zhai", "zhan",
    "zhang", "zhao", "zhe", "zhei", "zhen", "zheng", "zhi", "zhong", "zhou", "zhu", "zhua",
    "zhuai", "zhuan", "zhuang", "zhui", "zhun", "zhuo", "zi", "zong", "zou", "zu", "zuan", "zui",
    "zun", "zuo",
];

/// Returns `true` when `s` is exactly one toneless pinyin syllable.
pub fn is_syllable(s: &str) -> bool {
    SYLLABLES.contains(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_wellformed() {
        assert_eq!(SYLLABLES.len(), 425, "unexpected syllable inventory size");
        let mut sorted = SYLLABLES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), SYLLABLES.len(), "duplicate syllables");
        assert!(SYLLABLES.iter().all(|s| {
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase()) && s.len() <= 6
        }));
    }

    #[test]
    fn recognizes_common_syllables() {
        assert!(is_syllable("zhuang"));
        assert!(is_syllable("lv"));
        assert!(is_syllable("er"));
        assert!(is_syllable("a"));
        assert!(is_syllable("ri"));
    }

    #[test]
    fn recognizes_luna_dictionary_readings() {
        // Added so every key token in the bundled luna_pinyin dictionary
        // segments (notably `hou'xuan'ci` for 候选词). All readings come
        // from that data set; none are invented.
        for syllable in [
            "xuan", "zen", "kui", "dei", "nang", "eng", "lve", "nun", "nia", "pia", "zhei", "sei",
            "tei", "biang", "cei", "din", "fiao", "fong", "wong", "yai", "eh", "lvan",
        ] {
            assert!(is_syllable(syllable), "missing reading {syllable}");
        }
    }

    #[test]
    fn rejects_non_syllables() {
        assert!(!is_syllable("nihao"));
        assert!(!is_syllable(""));
        assert!(!is_syllable("zh"));
        assert!(!is_syllable("Ni"));
        assert!(!is_syllable("ni2"));
    }
}
