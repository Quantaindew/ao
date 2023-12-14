

use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::r2d2::ConnectionManager;
use diesel::r2d2::Pool;
use std::env::VarError;

use super::super::core::json::{Message, Process};
use super::super::router::{Scheduler, ProcessScheduler};
use crate::config::Config;

#[derive(Debug)]
pub enum StoreErrorType {
    DatabaseError(String),
    NotFound(String),
    JsonError(String),
    EnvVarError(String)
}

use diesel::result::Error as DieselError; // Import Diesel's Error

impl From<DieselError> for StoreErrorType {
    fn from(diesel_error: DieselError) -> Self {
        StoreErrorType::DatabaseError(format!("{:?}", diesel_error))
    }
}

impl From<serde_json::Error> for StoreErrorType {
    fn from(error: serde_json::Error) -> Self {
        StoreErrorType::JsonError(format!("data store json error: {}", error))
    }
}

impl From<StoreErrorType> for String {
    fn from(error: StoreErrorType) -> Self {
        format!("{:?}", error)
    }
}

impl From<VarError> for StoreErrorType {
    fn from(error: VarError) -> Self{
        StoreErrorType::EnvVarError(format!("data store env var error: {}", error))
    }
}

impl From<diesel::prelude::ConnectionError> for StoreErrorType {
    fn from(error: diesel::prelude::ConnectionError) -> Self{
        StoreErrorType::DatabaseError(format!("data store connection error: {}", error))
    }
}


pub struct StoreClient{
    pool: Pool<ConnectionManager<PgConnection>>
}

impl StoreClient {
    pub fn new() -> Result<Self, StoreErrorType> {
        let config = Config::new(Some("su".to_string())).expect("Failed to read configuration");
        let database_url = config.database_url;
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = Pool::builder()
            .test_on_check_out(true)
            .build(manager).map_err(
                |_| StoreErrorType::DatabaseError("Failed to initialize connection pool.".to_string())
            )?;

        Ok(StoreClient { pool })
    }

    pub fn get_conn(&self) -> Result<diesel::r2d2::PooledConnection<ConnectionManager<PgConnection>>, StoreErrorType> {
        self.pool.get().map_err(
            |_| StoreErrorType::DatabaseError("Failed to get connection from pool.".to_string())
        )
    }

    pub fn save_process(&self, process: &Process, bundle_in: &[u8]) -> Result<String, StoreErrorType> {
        use super::schema::processes::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let new_process = NewProcess {
            process_id: &process.process_id,
            process_data: serde_json::to_value(process).expect("Failed to serialize Process"),
            bundle: bundle_in
        };
    
        match diesel::insert_into(processes)
            .values(&new_process)
            .on_conflict(process_id)
            .do_nothing() 
            .execute(conn)
        {
            Ok(_) => {
                Ok("saved".to_string())
            },
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn get_process(&self, process_id_in: &str) -> Result<Process, StoreErrorType> {
        use super::schema::processes::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let db_process_result: Result<Option<DbProcess>, DieselError> = processes
            .filter(process_id.eq(process_id_in))
            .first(conn)
            .optional();
    
        match db_process_result {
            Ok(Some(db_process)) => {
                let process: Process = serde_json::from_value(db_process.process_data.clone())?;
                Ok(process)
            },
            Ok(None) => Err(StoreErrorType::NotFound("Process not found".to_string())), 
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }
    
    pub fn save_message(&self, message: &Message, bundle_in: &[u8]) -> Result<String, StoreErrorType> {
        use super::schema::messages::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let new_message = NewMessage {
            process_id: &message.process_id,
            message_id: &message.message.id,
            message_data: serde_json::to_value(message).expect("Failed to serialize Message"),
            epoch: &message.epoch,
            nonce: &message.nonce,
            timestamp: &message.timestamp,
            bundle: bundle_in,
            hash_chain: &message.hash_chain,
        };
    
        match diesel::insert_into(messages)
            .values(&new_message)
            .on_conflict(message_id)
            .do_nothing() 
            .execute(conn)
        {
            Ok(row_count) => {
                if row_count == 0 {
                    Err(StoreErrorType::DatabaseError("Duplicate message id".to_string())) // Return a custom error for duplicates
                } else {
                    Ok("saved".to_string())
                }
            },
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }    


    pub fn get_messages(&self, process_id_in: &str) -> Result<Vec<Message>, StoreErrorType> {
        use super::schema::messages::dsl::*;
        let conn = &mut self.get_conn()?;

        let db_messages_result: Result<Vec<DbMessage>, DieselError> = messages
            .filter(process_id.eq(process_id_in))
            .load(conn);

        match db_messages_result {
            Ok(db_messages) => {
                let n_messages: Result<Vec<Message>, StoreErrorType> = db_messages
                    .iter()
                    .map(|db_message| {
                        serde_json::from_value(db_message.message_data.clone())
                            .map_err(|e| StoreErrorType::from(e))
                    })
                    .collect();
        
                n_messages
            }
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn get_message(&self, message_id_in: &str) -> Result<Message, StoreErrorType> {
        use super::schema::messages::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let db_message_result: Result<Option<DbMessage>, DieselError> = messages
            .filter(message_id.eq(message_id_in))
            .first(conn)
            .optional();
    
        match db_message_result {
            Ok(Some(db_message)) => {
                let message: Message = serde_json::from_value(db_message.message_data.clone())?;
                Ok(message)
            },
            Ok(None) => Err(StoreErrorType::NotFound("Message not found".to_string())), // Adjust this error type as needed
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn get_latest_message(&self, process_id_in: &str) -> Result<Option<Message>, StoreErrorType> {
        use super::schema::messages::dsl::*;
        let conn = &mut self.get_conn()?;
    
        // Get the latest DbMessage
        let latest_db_message_result = messages
            .filter(process_id.eq(process_id_in))
            .order(row_id.desc())
            .first::<DbMessage>(conn);
    
        match latest_db_message_result {
            Ok(db_message) => {
                // Deserialize the message_data into Message
                let message = serde_json::from_value(db_message.message_data)
                    .map_err(|e| StoreErrorType::from(e))?;
    
                Ok(Some(message))
            },
            Err(DieselError::NotFound) => Ok(None), // No messages found
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }
    


    pub fn save_process_scheduler(&self, process_scheduler: &ProcessScheduler) -> Result<String, StoreErrorType> {
        use super::schema::process_schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let new_process_scheduler = NewProcessScheduler {
            process_id: &process_scheduler.process_id,
            scheduler_row_id: &process_scheduler.scheduler_row_id,
        };
    
        match diesel::insert_into(process_schedulers)
            .values(&new_process_scheduler)
            .on_conflict(process_id)
            .do_nothing() 
            .execute(conn)
        {
            Ok(_) => {
                Ok("saved".to_string())
            },
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn get_process_scheduler(&self, process_id_in: &str) -> Result<ProcessScheduler, StoreErrorType> {
        use super::schema::process_schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let db_process_result: Result<Option<DbProcessScheduler>, DieselError> = process_schedulers
            .filter(process_id.eq(process_id_in))
            .first(conn)
            .optional();
    
        match db_process_result {
            Ok(Some(db_process_scheduler)) => {
                let process_scheduler: ProcessScheduler = ProcessScheduler {
                    row_id: Some(db_process_scheduler.row_id),
                    process_id: db_process_scheduler.process_id,
                    scheduler_row_id: db_process_scheduler.scheduler_row_id,
                };
                Ok(process_scheduler)
            },
            Ok(None) => Err(StoreErrorType::NotFound("Process scheduler not found".to_string())), 
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn save_scheduler(&self, scheduler: &Scheduler) -> Result<String, StoreErrorType> {
        use super::schema::schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let new_scheduler = NewScheduler {
            url: &scheduler.url,
            process_count: &scheduler.process_count
        };
    
        match diesel::insert_into(schedulers)
            .values(&new_scheduler)
            .on_conflict(url)
            .do_nothing() 
            .execute(conn)
        {
            Ok(_) => {
                Ok("saved".to_string())
            },
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn update_scheduler(&self, scheduler: &Scheduler) -> Result<String, StoreErrorType> {
        use super::schema::schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        // Ensure scheduler.row_id is Some(value) before calling this function
        match diesel::update(schedulers.filter(row_id.eq(scheduler.row_id.unwrap())))
            .set((process_count.eq(scheduler.process_count), url.eq(&scheduler.url)))
            .execute(conn)
        {
            Ok(_) => Ok("updated".to_string()),
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }
    
    

    pub fn get_scheduler(&self, row_id_in: &i32) -> Result<Scheduler, StoreErrorType> {
        use super::schema::schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let db_scheduler_result: Result<Option<DbScheduler>, DieselError> = schedulers
            .filter(row_id.eq(row_id_in))
            .first(conn)
            .optional();
    
        match db_scheduler_result {
            Ok(Some(db_scheduler)) => {
                let scheduler: Scheduler = Scheduler {
                    row_id: Some(db_scheduler.row_id),
                    url: db_scheduler.url,
                    process_count: db_scheduler.process_count
                };
                Ok(scheduler)
            },
            Ok(None) => Err(StoreErrorType::NotFound("Scheduler not found".to_string())), 
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn get_scheduler_by_url(&self, url_in: &String) -> Result<Scheduler, StoreErrorType> {
        use super::schema::schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        let db_scheduler_result: Result<Option<DbScheduler>, DieselError> = schedulers
            .filter(url.eq(url_in))
            .first(conn)
            .optional();
    
        match db_scheduler_result {
            Ok(Some(db_scheduler)) => {
                let scheduler: Scheduler = Scheduler {
                    row_id: Some(db_scheduler.row_id),
                    url: db_scheduler.url,
                    process_count: db_scheduler.process_count
                };
                Ok(scheduler)
            },
            Ok(None) => Err(StoreErrorType::NotFound("Scheduler not found".to_string())), 
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }

    pub fn get_all_schedulers(&self) -> Result<Vec<Scheduler>, StoreErrorType> {
        use super::schema::schedulers::dsl::*;
        let conn = &mut self.get_conn()?;
    
        match schedulers.order(row_id.asc()).load::<DbScheduler>(conn) {
            Ok(db_schedulers) => {
                let schedulers_out: Vec<Scheduler> = db_schedulers.into_iter().map(|db_scheduler| {
                    Scheduler {
                        row_id: Some(db_scheduler.row_id),
                        url: db_scheduler.url,
                        process_count: db_scheduler.process_count
                    }
                }).collect();
                Ok(schedulers_out)
            },
            Err(e) => Err(StoreErrorType::from(e)),
        }
    }
    
    
}


#[derive(Queryable, Selectable)]
#[diesel(table_name = super::schema::processes)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DbProcess {
    pub row_id: i32,
    pub process_id: String,
    pub process_data: serde_json:: Value,
    pub bundle: Vec<u8>, 
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = super::schema::messages)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DbMessage {
    pub row_id: i32,
    pub process_id: String,
    pub message_id: String,
    pub message_data: serde_json::Value,
    pub epoch: i32,
    pub nonce: i32,
    pub timestamp: i64,
    pub bundle: Vec<u8>,
    pub hash_chain: String,
}


#[derive(Insertable)]
#[diesel(table_name = super::schema::messages)]
pub struct NewMessage<'a> {
    pub process_id: &'a str,
    pub message_id: &'a str,
    pub message_data: serde_json::Value,
    pub bundle:  &'a [u8],
    pub epoch: &'a i32,
    pub nonce: &'a i32,
    pub timestamp: &'a i64,
    pub hash_chain: &'a str,
}


#[derive(Insertable)]
#[diesel(table_name = super::schema::processes)]
pub struct NewProcess<'a> {
    pub process_id: &'a str,
    pub process_data: serde_json::Value,
    pub bundle:  &'a [u8],
}


#[derive(Queryable, Selectable)]
#[diesel(table_name = super::schema::schedulers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DbScheduler {
    pub row_id: i32,
    pub url: String,
    pub process_count: i32,
}


#[derive(Insertable)]
#[diesel(table_name = super::schema::schedulers)]
pub struct NewScheduler<'a> {
    pub url: &'a str,
    pub process_count: &'a i32,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = super::schema::process_schedulers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DbProcessScheduler {
    pub row_id: i32,
    pub process_id: String,
    pub scheduler_row_id: i32
}


#[derive(Insertable)]
#[diesel(table_name = super::schema::process_schedulers)]
pub struct NewProcessScheduler<'a> {
    pub process_id: &'a str,
    pub scheduler_row_id: &'a i32,
}