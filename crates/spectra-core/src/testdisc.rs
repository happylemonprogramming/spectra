//! Synthetic discs for tests: just enough ISO 9660 and UDF to be identified,
//! built in memory.

use crate::disc::{Disc, Media, SECTOR, Toc, Track};
use crate::{Error, Result};

pub struct MemDisc {
    pub sectors: Vec<u8>,
    pub media: Media,
    pub toc: Option<Toc>,
}

impl MemDisc {
    pub fn new(media: Media, sectors: usize) -> Self {
        Self {
            sectors: vec![0; sectors * SECTOR],
            media,
            toc: None,
        }
    }

    pub fn sector(&mut self, lba: usize) -> &mut [u8] {
        &mut self.sectors[lba * SECTOR..(lba + 1) * SECTOR]
    }

    /// An ISO 9660 volume: descriptor at 16, root directory at 18, file
    /// contents from 20, one sector each.
    pub fn iso(mut self, volume_id: &str, files: &[(&str, &[u8])], dirs: &[&str]) -> Self {
        let pvd = self.sector(16);
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[8..40].fill(b' ');
        pvd[40..72].fill(b' ');
        pvd[40..40 + volume_id.len()].copy_from_slice(volume_id.as_bytes());
        let total = (self.sectors.len() / SECTOR) as u32;
        self.sector(16)[80..84].copy_from_slice(&total.to_le_bytes());
        let root = record(18, SECTOR as u32, true, &[0]);
        self.sector(16)[156..156 + root.len()].copy_from_slice(&root);
        self.sector(17)[0] = 255;
        self.sector(17)[1..6].copy_from_slice(b"CD001");

        let mut dir = vec![];
        dir.extend(record(18, SECTOR as u32, true, &[0]));
        dir.extend(record(18, SECTOR as u32, true, &[1]));
        for (i, (name, data)) in files.iter().enumerate() {
            let lba = 20 + i;
            self.sector(lba)[..data.len()].copy_from_slice(data);
            dir.extend(record(
                lba as u32,
                data.len() as u32,
                false,
                format!("{name};1").as_bytes(),
            ));
        }
        for name in dirs {
            dir.extend(record(19, SECTOR as u32, true, name.as_bytes()));
        }
        self.sector(18)[..dir.len()].copy_from_slice(&dir);
        self
    }

    /// A UDF volume whose root holds `dirs`. With `metadata`, the directories
    /// live in a UDF 2.50 metadata partition, as on a Blu-ray.
    pub fn udf(mut self, label: &str, dirs: &[&str], metadata: bool) -> Self {
        const VDS: usize = 32;
        const PARTITION: u32 = 300;
        const METADATA_FILE: u32 = 10;
        const METADATA_START: u32 = 20;

        let anchor = self.sector(256);
        tag(anchor, 2);
        anchor[16..20].copy_from_slice(&(4 * SECTOR as u32).to_le_bytes());
        anchor[20..24].copy_from_slice(&(VDS as u32).to_le_bytes());

        tag(self.sector(VDS), 1);
        let pd = self.sector(VDS + 1);
        tag(pd, 5);
        pd[188..192].copy_from_slice(&PARTITION.to_le_bytes());

        let lvd = self.sector(VDS + 2);
        tag(lvd, 6);
        lvd[84] = 8;
        lvd[85..85 + label.len()].copy_from_slice(label.as_bytes());
        lvd[84 + 127] = label.len() as u8 + 1;
        let fsd_ref: u16 = if metadata { 1 } else { 0 };
        lvd[256..258].copy_from_slice(&fsd_ref.to_le_bytes());
        lvd[268..272].copy_from_slice(&(if metadata { 2u32 } else { 1 }).to_le_bytes());
        lvd[440] = 1;
        lvd[441] = 6;
        if metadata {
            let map = &mut lvd[446..446 + 64];
            map[0] = 2;
            map[1] = 64;
            map[5..5 + 23].copy_from_slice(b"*UDF Metadata Partition");
            map[40..44].copy_from_slice(&METADATA_FILE.to_le_bytes());
        }
        tag(self.sector(VDS + 3), 8);

        // Blocks within whichever partition holds the directories.
        let base = PARTITION + if metadata { METADATA_START } else { 0 };
        if metadata {
            let fe = self.sector((PARTITION + METADATA_FILE) as usize);
            file_entry(fe, 266, 16 * SECTOR as u32, METADATA_START);
        }
        let fsd = self.sector(base as usize);
        tag(fsd, 256);
        fsd[404..408].copy_from_slice(&1u32.to_le_bytes());
        fsd[408..410].copy_from_slice(&fsd_ref.to_le_bytes());

        let mut dir = fid("", true, true);
        for name in dirs {
            dir.extend(fid(name, true, false));
        }
        file_entry(self.sector(base as usize + 1), 261, dir.len() as u32, 2);
        self.sector(base as usize + 2)[..dir.len()].copy_from_slice(&dir);
        self
    }

    pub fn with_toc(mut self, tracks: &[(u32, bool)], leadout: u32) -> Self {
        self.toc = Some(Toc {
            first: 1,
            last: tracks.len() as u8,
            tracks: tracks
                .iter()
                .enumerate()
                .map(|(i, &(lba, data))| Track {
                    number: i as u8 + 1,
                    lba,
                    data,
                })
                .collect(),
            leadout,
        });
        self
    }
}

fn tag(sector: &mut [u8], id: u16) {
    sector[..2].copy_from_slice(&id.to_le_bytes());
}

/// A (extended) file entry with one short allocation descriptor.
fn file_entry(sector: &mut [u8], id: u16, size: u32, position: u32) {
    tag(sector, id);
    sector[56..60].copy_from_slice(&size.to_le_bytes());
    let ad = if id == 266 { 216 } else { 176 };
    let lengths = if id == 266 { 208 } else { 168 };
    sector[lengths + 4..lengths + 8].copy_from_slice(&8u32.to_le_bytes());
    sector[ad..ad + 4].copy_from_slice(&size.to_le_bytes());
    sector[ad + 4..ad + 8].copy_from_slice(&position.to_le_bytes());
}

fn fid(name: &str, directory: bool, parent: bool) -> Vec<u8> {
    let name: Vec<u8> = if name.is_empty() {
        vec![]
    } else {
        [&[8u8][..], name.as_bytes()].concat()
    };
    let mut f = vec![0u8; 38];
    tag(&mut f, 257);
    f[18] = (u8::from(directory) * 2) | (u8::from(parent) * 8);
    f[19] = name.len() as u8;
    f.extend(&name);
    f.resize((f.len() + 3) & !3, 0);
    f
}

pub fn record(lba: u32, size: u32, directory: bool, name: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; 33];
    r[2..6].copy_from_slice(&lba.to_le_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[25] = if directory { 2 } else { 0 };
    r[32] = name.len() as u8;
    r.extend(name);
    if r.len() % 2 == 1 {
        r.push(0);
    }
    r[0] = r.len() as u8;
    r
}

impl Disc for MemDisc {
    fn describe(&self) -> String {
        "memory".into()
    }

    fn media(&mut self) -> Result<Media> {
        Ok(self.media)
    }

    fn toc(&mut self) -> Result<Option<Toc>> {
        Ok(self.toc.clone())
    }

    fn read(&mut self, lba: u32, count: u32) -> Result<Vec<u8>> {
        let (from, to) = (lba as usize * SECTOR, (lba + count) as usize * SECTOR);
        self.sectors
            .get(from..to)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| Error::format("past the end"))
    }

    fn capacity(&mut self) -> Result<Option<u32>> {
        Ok(Some((self.sectors.len() / SECTOR) as u32))
    }
}

mod tests {
    use super::MemDisc;
    use crate::identify::{DiscKind, GameSystem, identify};
    use crate::{Disc, Media};

    fn kind(disc: &mut dyn Disc) -> DiscKind {
        identify(disc).unwrap().kind
    }

    fn system(disc: &mut dyn Disc) -> GameSystem {
        match kind(disc) {
            DiscKind::Game(g) => g.system,
            other => panic!("not a game: {other:?}"),
        }
    }

    #[test]
    fn ps2_with_title() {
        let cnf = b"BOOT2 = cdrom0:\\SCUS_971.11;1\r\nVER = 1.00\r\nVMODE = NTSC\r\n";
        let mut disc = MemDisc::new(Media::Dvd, 64).iso(
            "DARK_CLOUD",
            &[("SYSTEM.CNF", cnf), ("SCUS_971.11", b"ELF")],
            &[],
        );
        let report = identify(&mut disc).unwrap();
        let DiscKind::Game(game) = report.kind else {
            panic!()
        };
        assert_eq!(game.system, GameSystem::Ps2);
        assert_eq!(game.serial.as_deref(), Some("SCUS-97111"));
        assert_eq!(game.title.as_deref(), Some("Dark Cloud"));
        assert_eq!(game.region.as_deref(), Some("NTSC-U"));
        assert_eq!(report.label.as_deref(), Some("DARK_CLOUD"));
    }

    #[test]
    fn ps1() {
        let mut disc = MemDisc::new(Media::Cd, 64)
            .iso(
                "",
                &[("SYSTEM.CNF", b"BOOT = cdrom:\\SLUS_000.67;1\nTCB = 4\n")],
                &[],
            )
            .with_toc(&[(0, true)], 64);
        let DiscKind::Game(game) = kind(&mut disc) else {
            panic!()
        };
        assert_eq!(
            (game.system, game.serial.as_deref()),
            (GameSystem::Ps1, Some("SLUS-00067"))
        );
    }

    #[test]
    fn saturn_header() {
        let mut disc = MemDisc::new(Media::Cd, 32);
        let s = disc.sector(0);
        s[..16].copy_from_slice(b"SEGA SEGASATURN ");
        s[0x20..0x2a].copy_from_slice(b"MK-81009  ");
        s[0x60..0x6e].copy_from_slice(b"NIGHTS        ");
        let DiscKind::Game(game) = kind(&mut disc) else {
            panic!()
        };
        assert_eq!(game.system, GameSystem::Saturn);
        assert_eq!(game.serial.as_deref(), Some("MK-81009"));
        assert_eq!(game.title.as_deref(), Some("NIGHTS"));
    }

    #[test]
    fn pc_engine_boots_from_its_second_track() {
        let mut disc =
            MemDisc::new(Media::Cd, 400).with_toc(&[(0, false), (300, true), (390, false)], 400);
        disc.sector(301)[0x20..0x20 + 23].copy_from_slice(b"PC Engine CD-ROM SYSTEM");
        assert_eq!(system(&mut disc), GameSystem::PcEngineCd);
    }

    #[test]
    fn gamecube_and_wii() {
        let mut gc = MemDisc::new(Media::Dvd, 64);
        gc.sector(0)[..6].copy_from_slice(b"GALE01");
        gc.sector(0)[0x1c..0x20].copy_from_slice(&[0xc2, 0x33, 0x9f, 0x3d]);
        gc.sector(0)[0x20..0x32].copy_from_slice(b"Super Smash Bros. ");
        let DiscKind::Game(game) = kind(&mut gc) else {
            panic!()
        };
        assert_eq!(
            (game.system, game.serial.as_deref()),
            (GameSystem::GameCube, Some("GALE01"))
        );

        let mut wii = MemDisc::new(Media::Dvd, 64);
        wii.sector(0)[0x18..0x1c].copy_from_slice(&[0x5d, 0x1c, 0x9e, 0xa3]);
        assert_eq!(system(&mut wii), GameSystem::Wii);
    }

    #[test]
    fn xbox_iso() {
        let mut disc = MemDisc::new(Media::Dvd, 64);
        disc.sector(32)[..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
        assert_eq!(system(&mut disc), GameSystem::Xbox);
    }

    #[test]
    fn ps3_and_neo_geo() {
        let mut ps3 = MemDisc::new(Media::BluRay, 64).iso(
            "PS3VOLUME",
            &[("PS3_DISC.SFB", b".SFB")],
            &["PS3_GAME"],
        );
        assert_eq!(system(&mut ps3), GameSystem::Ps3);
        let mut neo = MemDisc::new(Media::Cd, 64).iso("", &[("IPL.TXT", b"PROG.PRG,0,0")], &[]);
        assert_eq!(system(&mut neo), GameSystem::NeoGeoCd);
    }

    #[test]
    fn dvd_video_through_udf() {
        let mut disc =
            MemDisc::new(Media::Dvd, 400).udf("THE_MOVIE", &["VIDEO_TS", "AUDIO_TS"], false);
        let report = identify(&mut disc).unwrap();
        assert_eq!(
            report.kind,
            DiscKind::DvdVideo {
                feature_seconds: None
            }
        );
        assert_eq!(report.label.as_deref(), Some("THE_MOVIE"));
    }

    #[test]
    fn dvd_video_through_iso_bridge() {
        let mut disc = MemDisc::new(Media::Dvd, 64).iso("THE_MOVIE", &[], &["VIDEO_TS"]);
        assert_eq!(
            kind(&mut disc),
            DiscKind::DvdVideo {
                feature_seconds: None
            }
        );
    }

    #[test]
    fn blu_ray_through_metadata_partition() {
        let mut disc = MemDisc::new(Media::BluRay, 400).udf("FILM", &["BDMV", "CERTIFICATE"], true);
        let report = identify(&mut disc).unwrap();
        assert_eq!(report.kind, DiscKind::BluRayVideo);
        assert_eq!(report.label.as_deref(), Some("FILM"));
    }

    #[test]
    fn video_cd_pc_and_plain_data() {
        let mut vcd = MemDisc::new(Media::Cd, 64).iso("", &[], &["VCD", "MPEGAV"]);
        assert_eq!(kind(&mut vcd), DiscKind::VideoCd { super_vcd: false });
        let mut pc = MemDisc::new(Media::Cd, 64).iso(
            "BW",
            &[("AUTORUN.INF", b"[autorun]\r\nopen=setup.exe\r\n")],
            &[],
        );
        assert_eq!(kind(&mut pc), DiscKind::Pc);
        let mut data = MemDisc::new(Media::Cd, 64).iso("BACKUP", &[("NOTES.TXT", b"hi")], &[]);
        assert_eq!(kind(&mut data), DiscKind::Data);
    }

    #[test]
    fn audio_and_enhanced_cd() {
        let mut audio = MemDisc::new(Media::Cd, 1).with_toc(&[(0, false), (15_000, false)], 30_000);
        let DiscKind::Audio {
            tracks,
            enhanced,
            musicbrainz,
        } = kind(&mut audio)
        else {
            panic!()
        };
        assert_eq!((tracks, enhanced), (2, false));
        assert!(musicbrainz.is_some());

        let mut enhanced = MemDisc::new(Media::Cd, 1)
            .with_toc(&[(0, false), (15_000, false), (41_400, true)], 60_000);
        assert!(matches!(
            kind(&mut enhanced),
            DiscKind::Audio {
                tracks: 2,
                enhanced: true,
                ..
            }
        ));
    }
}
