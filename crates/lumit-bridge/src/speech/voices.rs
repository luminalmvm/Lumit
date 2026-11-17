//! Telling the voices of a whole job apart, so one person is one speaker
//! in every file the job hears.

use super::*;

#[derive(Default)]
pub(super) struct Voices {
    /// Every file's voices so far, in the order the files were heard.
    heard: Vec<Vec<lumit_ml::speech::Voice>>,
    /// The speaker each of those voices was given when its file landed.
    given: Vec<Vec<Uuid>>,
    /// Every speaker made, in order, with their name.
    names: Vec<(Uuid, String)>,
    /// The one speaker of a job asked to hear one voice.
    one: Option<Uuid>,
}

impl Voices {
    /// The speakers for a newly heard file's voices. Every file so far is
    /// clustered together; a cluster that already holds voices given a
    /// speaker keeps that speaker, and a new one gets a new speaker.
    pub(super) fn speakers_for(
        &mut self,
        voices: Vec<lumit_ml::speech::Voice>,
        count: Option<u32>,
        template: &str,
    ) -> Vec<Uuid> {
        self.heard.push(voices);
        let all: Vec<&[lumit_ml::speech::Voice]> = self.heard.iter().map(Vec::as_slice).collect();
        let clusters = lumit_ml::speech::cluster(&all, count);
        // The speakers already given to each cluster, by votes.
        let mut votes: BTreeMap<u32, BTreeMap<Uuid, usize>> = BTreeMap::new();
        for (file, given) in self.given.iter().enumerate() {
            for (voice, speaker) in given.iter().enumerate() {
                if let Some(cluster) = clusters.get(file).and_then(|c| c.get(voice)) {
                    *votes
                        .entry(*cluster)
                        .or_default()
                        .entry(*speaker)
                        .or_default() += 1;
                }
            }
        }
        let mut chosen: BTreeMap<u32, Uuid> = BTreeMap::new();
        let newest = clusters.last().cloned().unwrap_or_default();
        let ids: Vec<Uuid> = newest
            .iter()
            .map(|cluster| {
                if let Some(id) = chosen.get(cluster) {
                    return *id;
                }
                let id = votes
                    .get(cluster)
                    .and_then(|v| v.iter().max_by_key(|(_, n)| **n).map(|(id, _)| *id))
                    .unwrap_or_else(|| self.make(template));
                chosen.insert(*cluster, id);
                id
            })
            .collect();
        self.given.push(ids.clone());
        ids
    }

    /// The single speaker of a job asked to hear one voice.
    pub(super) fn only(&mut self, template: &str) -> Uuid {
        if let Some(one) = self.one {
            return one;
        }
        let one = self.make(template);
        self.one = Some(one);
        one
    }

    fn make(&mut self, template: &str) -> Uuid {
        let id = Uuid::now_v7();
        let number = self.names.len() + 1;
        let name = if template.contains("{n}") {
            template.replace("{n}", &number.to_string())
        } else {
            format!("{template} {number}")
        };
        self.names.push((id, name.trim().to_owned()));
        id
    }

    /// The speakers `used`, named, in the order they were made.
    pub(super) fn named(&self, used: &[Uuid]) -> Vec<Speaker> {
        self.names
            .iter()
            .filter(|(id, _)| used.contains(id))
            .map(|(id, name)| Speaker {
                id: *id,
                name: name.clone(),
            })
            .collect()
    }
}
