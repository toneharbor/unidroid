use super::*;
use crate::Resume;

pub(super) enum Question<'a> {
    Jni { kind: u8, context: &'a str },
    Stop,
}

pub(super) enum Reply {
    Jni(Option<JniValue>),
    Resume(Resume),
}

impl<R: BufRead, W: Write> Session<R, W> {
    pub(super) fn ask(
        &mut self,
        emu: &mut Emulator,
        response: Response,
        question: Question<'_>,
    ) -> Option<Reply> {
        if !self.driver {
            return None;
        }
        match self.wait(emu, response, question) {
            Ok(reply) => Some(reply),
            Err(error) => {
                self.error = Some(error);
                self.fatal = true;
                None
            }
        }
    }

    fn wait(
        &mut self,
        emu: &mut Emulator,
        response: Response,
        question: Question<'_>,
    ) -> Result<Reply> {
        self.write(response)?;
        loop {
            let command = self.read()?.ok_or(match question {
                Question::Jni { .. } => {
                    "stdin closed while a JNI question was waiting for an answer"
                }
                Question::Stop => {
                    "stdin closed while a breakpoint stop was waiting for a resume command"
                }
            })?;
            let error = match command {
                Ok(Command::Answer { value }) => match question {
                    Question::Jni { kind, context } => {
                        return Ok(Reply::Jni(match decode(kind, &value, context) {
                            Ok(value) => Some(value),
                            Err(error) => {
                                self.error = Some(error);
                                None
                            }
                        }));
                    }
                    Question::Stop => {
                        "breakpoint stop expects continue, step, finish, return or abort"
                    }
                },
                Ok(Command::Unhandled) => match question {
                    Question::Jni { .. } => return Ok(Reply::Jni(None)),
                    Question::Stop => {
                        "breakpoint stop expects continue, step, finish, return or abort"
                    }
                },
                Ok(
                    command @ (Command::Continue
                    | Command::Step
                    | Command::Finish
                    | Command::Return { .. }
                    | Command::Abort),
                ) => match question {
                    Question::Stop => {
                        return Ok(Reply::Resume(match command {
                            Command::Continue => Resume::Continue,
                            Command::Step => Resume::Step,
                            Command::Finish => Resume::Finish,
                            Command::Return { value } => Resume::Return(value.0),
                            Command::Abort => Resume::Abort,
                            _ => unreachable!(),
                        }));
                    }
                    Question::Jni { .. } => "JNI question expects answer or unhandled",
                },
                command => {
                    self.respond(emu, command)?;
                    continue;
                }
            };
            self.write(Response::Error {
                error: error.into(),
                crash: None,
            })?;
        }
    }
}
