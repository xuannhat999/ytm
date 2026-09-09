use data::app::{PlayListPrivacy, Playlist, Song};
use error::{
    YError::{self},
    YResult,
};

use crate::{dao::YTDao, parser};

pub struct YTBus {
    dao: YTDao,
}

impl YTBus {
    pub fn new(dao: YTDao) -> Self {
        Self { dao }
    }

    pub async fn create_playlist(
        &self,
        title: &str,
        desc: &str,
        privacy: PlayListPrivacy,
    ) -> YResult<Playlist> {
        self.check_auth()?;
        let res = self.dao.create_playlist_raw(title, desc, privacy).await?;
        let playlist = parser::parse_created_playlist(&res)?;
        Ok(playlist)
    }

    pub async fn get_lists(&self) -> YResult<(Vec<Playlist>, Vec<Playlist>, Vec<usize>)> {
        self.check_auth()?;
        let mut all_albums: Vec<Playlist> = Vec::new();
        let mut all_playlists: Vec<Playlist> = Vec::new();
        let mut all_cus_playlists: Vec<usize> = Vec::new();
        let raw_data = self.dao.get_raw_lists().await?;

        let (mut albums, mut playlists, mut token) = parser::parse_lists(&raw_data)?;
        all_albums.append(&mut albums);
        all_playlists.append(&mut playlists);
        while let Some(current_token) = token {
            let next_raw_data = self.dao.get_continuation_raw(&current_token).await?;
            let (mut next_albums, mut next_playlists, next_token) =
                parser::parse_lists(&next_raw_data)?;
            all_albums.append(&mut next_albums);
            all_playlists.append(&mut next_playlists);
            token = next_token;
        }
        for (idx, playlist) in all_playlists.iter_mut().enumerate() {
            if playlist.playlist_id == "LM" {
                playlist.is_custom = true;
            }
            if playlist.is_custom {
                all_cus_playlists.push(idx);
            }
        }
        Ok((all_albums, all_playlists, all_cus_playlists))
    }

    pub async fn get_songs(&self, browse_id: &str) -> YResult<Vec<Song>> {
        let raw = self.dao.get_songs_raw(browse_id).await?;
        parser::parse_songs(&raw)
    }

    pub async fn get_search_albums(&self, query: &str) -> YResult<Vec<Playlist>> {
        let raw_list = self.dao.search_with_params_raw(query, 2).await?;
        parser::parse_search_albums(&raw_list)
    }

    pub async fn get_search_songs(&self, query: &str) -> YResult<Vec<Song>> {
        let top_res_raw = self.dao.search_raw(query).await?;
        let raw_songs = self.dao.search_with_params_raw(query, 1).await?;
        let mut songs = parser::parse_search_songs(&raw_songs)?;
        if let Ok(top_res) = parser::parse_top_songs(&top_res_raw) {
            let mut seen: Vec<String> = songs.iter().map(|s| s.video_id.clone()).collect();
            for song in top_res.into_iter().rev() {
                if !seen.contains(&song.video_id) {
                    seen.push(song.video_id.clone());
                    songs.insert(0, song);
                }
            }
        }
        Ok(songs)
    }

    pub async fn get_params(&self, video_id: &str) -> YResult<String> {
        let raw_data = self.dao.get_params_raw(video_id).await?;
        parser::parse_params(&raw_data)
    }

    pub async fn get_related_songs(&self, song: Song, params: &str) -> YResult<Vec<Song>> {
        let video_id = &song.video_id;
        let playlist_id = format!("RDAMVM{}", video_id);
        let raw_data = self.dao.get_related_songs_raw(&playlist_id, params).await?;
        let mut songs = parser::parse_related_songs(&raw_data)?;
        songs.insert(0, song);
        Ok(songs)
    }

    pub async fn save_to_playlist(&self, song: &Song, playlist_id: &str) -> YResult<()> {
        self.check_auth()?;
        self.dao.save_to_playlist_raw(song, playlist_id).await
    }

    pub async fn unsave_to_playlist(&self, song: &Song, playlist_id: &str) -> YResult<()> {
        self.check_auth()?;
        self.dao.unsave_to_playlist_raw(song, playlist_id).await
    }

    pub async fn like_song(&self, song: &Song) -> YResult<()> {
        self.check_auth()?;
        self.dao.like_song_raw(song).await
    }

    pub async fn unlike_song(&self, song: &Song) -> YResult<()> {
        self.check_auth()?;
        self.dao.unlike_song_raw(song).await
    }

    pub async fn save_album(&self, playlist_id: &str) -> YResult<()> {
        self.check_auth()?;
        self.dao.save_album_raw(playlist_id).await
    }

    pub async fn unsave_album(&self, playlist_id: &str) -> YResult<()> {
        self.check_auth()?;
        self.dao.unsave_album_raw(playlist_id).await
    }

    pub async fn unsave_cus_playlist(&self, playlist_id: &str) -> YResult<()> {
        self.check_auth()?;
        self.dao.unsave_cus_playlist_raw(playlist_id).await
    }
    fn check_auth(&self) -> YResult<()> {
        if self.dao.sapisid.is_none() {
            return Err(YError::UnavailableFeature);
        }
        Ok(())
    }
}
