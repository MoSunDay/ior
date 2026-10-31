//! Toneless Mandarin pinyin syllable table.
//!
//! The table holds the ~404 base syllables used by the M0 engine (no tone
//! suffixes, `lv`/`nv` spellings kept for `lue`/`nve`). Segmentation,
//! dictionary key building and dictionary loading all consult this single
//! source of truth through [`is_syllable`].

/// All toneless Mandarin pinyin syllables (lowercase, `v` spellings for
/// `u:`-umlaut). Roughly grouped by initial consonant.
pub const SYLLABLES: &[&str] = &[
    "a", "ai", "an", "ang", "ao", "ba", "bai", "ban", "bang", "bao", "bei", "ben", "beng", "bi",
    "bian", "biao", "bie", "bin", "bing", "bo", "bu", "ca", "cai", "can", "cang", "cao", "ce",
    "cen", "ceng", "cha", "chai", "chan", "chang", "chao", "che", "chen", "cheng", "chi", "chong",
    "chou", "chu", "chua", "chuai", "chuan", "chuang", "chui", "chun", "chuo", "ci", "cong", "cou",
    "cu", "cuan", "cui", "cun", "cuo", "da", "dai", "dan", "dang", "dao", "de", "den", "deng",
    "di", "dia", "dian", "diao", "die", "ding", "diu", "dong", "dou", "du", "duan", "dui", "dun",
    "duo", "e", "ei", "en", "er", "fa", "fan", "fang", "fei", "fen", "feng", "fo", "fou", "fu",
    "ga", "gai", "gan", "gang", "gao", "ge", "gei", "gen", "geng", "gong", "gou", "gu", "gua",
    "guai", "guan", "guang", "gui", "gun", "guo", "ha", "hai", "han", "hang", "hao", "he", "hei",
    "hen", "heng", "hong", "hou", "hu", "hua", "huai", "huan", "huang", "hui", "hun", "huo", "ji",
    "jia", "jian", "jiang", "jiao", "jie", "jin", "jing", "jiong", "jiu", "ju", "juan", "jue",
    "jun", "ka", "kai", "kan", "kang", "kao", "ke", "kei", "ken", "keng", "kong", "kou", "ku",
    "kua", "kuai", "kuan", "kuang", "kun", "kuo", "la", "lai", "lan", "lang", "lao", "le", "lei",
    "leng", "li", "lia", "lian", "liang", "liao", "lie", "lin", "ling", "liu", "lo", "long", "lou",
    "lu", "luan", "lun", "luo", "lv", "lue", "ma", "mai", "man", "mang", "mao", "me", "mei", "men",
    "meng", "mi", "mian", "miao", "mie", "min", "ming", "miu", "mo", "mou", "mu", "na", "nai",
    "nan", "nao", "ne", "nei", "nen", "neng", "ni", "nian", "niang", "niao", "nie", "nin", "ning",
    "niu", "nong", "nou", "nu", "nuan", "nuo", "nv", "nve", "o", "ou", "pa", "pai", "pan", "pang",
    "pao", "pei", "pen", "peng", "pi", "pian", "piao", "pie", "pin", "ping", "po", "pou", "pu",
    "qi", "qia", "qian", "qiang", "qiao", "qie", "qin", "qing", "qiong", "qiu", "qu", "quan",
    "que", "qun", "ran", "rang", "rao", "re", "ren", "reng", "ri", "rong", "rou", "ru", "rua",
    "ruan", "rui", "run", "ruo", "sa", "sai", "san", "sang", "sao", "se", "sen", "seng", "sha",
    "shai", "shan", "shang", "shao", "she", "shei", "shen", "sheng", "shi", "shou", "shu", "shua",
    "shuai", "shuan", "shuang", "shui", "shun", "shuo", "si", "song", "sou", "su", "suan", "sui",
    "sun", "suo", "ta", "tai", "tan", "tang", "tao", "te", "teng", "ti", "tian", "tiao", "tie",
    "ting", "tong", "tou", "tu", "tuan", "tui", "tun", "tuo", "wa", "wai", "wan", "wang", "wei",
    "wen", "weng", "wo", "wu", "xi", "xia", "xian", "xiang", "xiao", "xie", "xin", "xing", "xiong",
    "xiu", "xu", "xue", "xun", "ya", "yan", "yang", "yao", "ye", "yi", "yin", "ying", "yo", "yong",
    "you", "yu", "yuan", "yun", "yue", "za", "zai", "zan", "zang", "zao", "ze", "zei", "zeng",
    "zha", "zhai", "zhan", "zhang", "zhao", "zhe", "zhen", "zheng", "zhi", "zhong", "zhou", "zhu",
    "zhua", "zhuai", "zhuan", "zhuang", "zhui", "zhun", "zhuo", "zi", "zong", "zou", "zu", "zuan",
    "zui", "zun", "zuo",
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
        assert!(
            SYLLABLES.len() >= 400,
            "table too small: {}",
            SYLLABLES.len()
        );
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
    fn rejects_non_syllables() {
        assert!(!is_syllable("nihao"));
        assert!(!is_syllable(""));
        assert!(!is_syllable("zh"));
        assert!(!is_syllable("Ni"));
        assert!(!is_syllable("ni2"));
    }
}
