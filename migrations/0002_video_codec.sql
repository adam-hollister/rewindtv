-- Source video codec, so browser playback only remuxes what browsers can play.
ALTER TABLE media_items ADD COLUMN video_codec TEXT;
